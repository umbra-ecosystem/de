//! Jira adapter over Atlassian's `acli` (`acli jira workitem ...`).
//!
//! [`AcliJira`] implements [`TicketProvider`] (reads only) and [`AcliJiraWriter`]
//! implements [`TicketWriter`] (writes only); the registry builds them separately.
//! Both run `acli` through a [`CommandRunner`] (a [`ProcessRunner`] in production) and never
//! handle credentials: `acli` owns its login.
//!
//! # Commands used, and where each comes from
//!
//! Verified against a real `acli 1.3.39-stable` (read commands only). Writes cannot be
//! verified without changing real Jira, so they stay as documented.
//!
//! | Purpose | Invocation | Status |
//! |---|---|---|
//! | version | `acli --version` (prints `acli version 1.3.39-stable`) | VERIFIED |
//! | auth check | `acli jira auth status` (exit 0 and `Authenticated` when logged in) | VERIFIED logged in; logged-out wording UNVERIFIED |
//! | search | `acli jira workitem search --jql Q --paginate --json --fields key,summary,status,priority,assignee` | VERIFIED. `updated` is **rejected** by search (`field 'updated' is not allowed`); an unbounded JQL (only an `order by`) is rejected too |
//! | view | `acli jira workitem view KEY --json --fields key,summary,status,priority,assignee,updated` | VERIFIED; the only path that offers `updated` |
//! | comments | `acli jira workitem view KEY --json --fields comment` | VERIFIED: `fields.comment.comments[]` with author account ids, ADF bodies and timestamps |
//! | add comment | `acli jira workitem comment create --key KEY --body TEXT --json` | flags documented; the JSON it prints UNVERIFIED |
//! | transition | `acli jira workitem transition --key KEY --status NAME --yes --json` | flags documented; UNVERIFIED |
//!
//! `acli jira workitem comment list` is **not used**: it prints only
//! `{author: "<display name>", body: "<plain text>", id, visibility}`, with no account id,
//! no timestamp and no mention data, so it cannot answer "was I @mentioned?".
//!
//! `acli` has no command to list the transitions available for a work item, so
//! [`AcliJiraWriter::transition`] cannot enumerate them (it reports acli's own error).
//!
//! # `updated_at` from search
//!
//! Search cannot return `updated`, so [`RemoteTicket::updated_at`] is
//! [`UPDATED_UNKNOWN`] (`0`) for search results and real for [`TicketProvider::get`]. Sync
//! does not store or compare it (the Jira cache has no such column).
//!
//! # Errors
//!
//! `acli` prints errors as `✗ Error: ...`, on stderr for most commands but on **stdout**
//! for `comment list` (with a generic line on stderr). Any non-zero exit is an error and
//! its classification looks at both streams; an exit 0 whose stdout starts with `✗` is also
//! treated as an error. `Issue does not exist or you do not have permission to see it.`
//! is [`ProviderError::NotFound`].
//!
//! `acli` 1.3.4 (19 September 2025) introduced `comment list`; [`MIN_ACLI_VERSION`] is kept
//! at that floor.

mod adf;
mod time;
mod wire;

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use super::error::{ProviderError, ProviderResult};
use super::model::{Health, RemoteComment, RemoteTicket};
use super::traits::{TicketProvider, TicketWriter};
use crate::config::Config;
use crate::domain::TicketKey;
use crate::overlay::{CommandOutput, CommandRunner, ExternalCommand, ProcessRunner, TimedOut};

pub use adf::{adf_to_text, body_to_text};
pub use time::parse_timestamp;
use wire::{TOOL, snippet};

/// The oldest `acli` this adapter is written for: 1.3.4, the first release with
/// `acli jira workitem comment list` (changelog entry of 19 September 2025). Older versions
/// lack the command entirely, so they are reported as not meeting the minimum.
pub const MIN_ACLI_VERSION: &str = "1.3.4";

const PROGRAM: &str = "acli";

/// Fields requested from search. Verified against a real acli: `updated` is NOT allowed
/// here, and `--fields key` alone yields nulls, so `summary` is always included.
const SEARCH_FIELDS: &str = "key,summary,status,priority,assignee";

/// Fields requested from view, where `updated` is allowed.
const VIEW_FIELDS: &str = "key,summary,status,priority,assignee,updated";

/// Field requested to read comments.
const COMMENT_FIELDS: &str = "comment";

/// The value of [`RemoteTicket::updated_at`] when the source cannot say.
pub const UPDATED_UNKNOWN: i64 = 0;

// ---- the shared command layer ---------------------------------------------------------

struct Acli<R> {
    runner: R,
    site: Option<String>,
}

impl<R: CommandRunner> Acli<R> {
    fn command(args: &[&str]) -> ExternalCommand {
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let label = args
            .iter()
            .take_while(|a| !a.starts_with('-'))
            .copied()
            .fold(String::from(PROGRAM), |acc, a| format!("{acc} {a}"));
        ExternalCommand::new(dir, PROGRAM, args, label)
    }

    /// Runs `acli` and returns its output on success. A missing binary, a failed exit and
    /// the environmental failures are mapped to distinct [`ProviderError`]s.
    fn run(&self, args: &[&str]) -> ProviderResult<CommandOutput> {
        let output = self.run_raw(args)?;
        // Some acli failures print `✗ Error: ...` and still exit 0.
        if output.success && !output.stdout.trim_start().starts_with('✗') {
            Ok(output)
        } else {
            Err(classify_failure(&output))
        }
    }

    fn run_raw(&self, args: &[&str]) -> ProviderResult<CommandOutput> {
        self.runner
            .run(&Self::command(args))
            .map_err(|e| spawn_error(&e))
    }
}

fn is_not_found(e: &eyre::Report) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::NotFound)
    })
}

/// How long one `acli` call may run before it is killed (a hung acli must not hang sync).
pub const ACLI_TIMEOUT: Duration = Duration::from_secs(120);

fn spawn_error(e: &eyre::Report) -> ProviderError {
    if let Some(t) = e.chain().find_map(|c| c.downcast_ref::<TimedOut>()) {
        // Environmental: sync stops talking to Jira and keeps its cache.
        ProviderError::Network(format!("acli did not answer: {t}"))
    } else if is_not_found(e) {
        ProviderError::not_installed(TOOL, "`acli` was not found on PATH")
    } else {
        ProviderError::Command {
            tool: TOOL.into(),
            status: None,
            stderr: format!("{e:#}"),
        }
    }
}

/// Whether `code` (an HTTP status) appears as a word of its own and not as part of a ticket
/// key or a number (`PROJ-401`, `#404`, a path segment, `v1.401`).
fn has_status_code(text: &str, code: &str) -> bool {
    text.match_indices(code).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + code.len()..].chars().next();
        !before
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '#' | '/' | '_' | '.'))
            && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Turns a failed `acli` run into the error sync can act on. Matches on wording because
/// `acli` documents no exit codes (UNVERIFIED): auth first, then network, then not found.
fn classify_failure(output: &CommandOutput) -> ProviderError {
    // `acli` prints the real message on stdout for some commands and only a generic line
    // on stderr; prefer whichever stream carries the message.
    let detail = if output.stderr.trim().is_empty()
        || (output.stderr.contains("command execution failed") && !output.stdout.trim().is_empty())
    {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    let lower = format!("{}\n{}", output.stderr, output.stdout).to_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if any(&[
        "not logged in",
        "not authenticated",
        "unauthorized",
        "unauthenticated",
        "authentication required",
        "auth login",
        "session expired",
        "invalid credentials",
    ]) || has_status_code(&lower, "401")
    {
        ProviderError::not_authenticated(TOOL, snippet(detail, 300))
    } else if any(&[
        "no such host",
        "connection refused",
        "timed out",
        "i/o timeout",
        "could not resolve",
        "network is unreachable",
        "dial tcp",
        "temporary failure in name resolution",
    ]) {
        ProviderError::Network(snippet(detail, 300))
    } else if (any(&["does not exist", "do not have permission", "not found"])
        && !any(&["command not found"]))
        || has_status_code(&lower, "404")
    {
        ProviderError::NotFound(snippet(detail, 300))
    } else {
        ProviderError::Command {
            tool: TOOL.into(),
            status: output.code,
            stderr: snippet(detail, 500),
        }
    }
}

fn site_of(config: &Config) -> Option<String> {
    config.jira.as_ref().and_then(|j| j.site.clone())
}

/// `(major, minor, patch)` from the first `x.y[.z]` in `text` (`acli version 1.3.4-stable`).
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    for word in text.split(|c: char| c.is_whitespace() || c == 'v' || c == ',' || c == '(') {
        let mut parts = word.trim().split('.');
        let numeric = |p: Option<&str>| -> Option<u64> {
            let p = p?;
            let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        };
        if let (Some(a), Some(b)) = (numeric(parts.next()), numeric(parts.next())) {
            return Some((a, b, numeric(parts.next()).unwrap_or(0)));
        }
    }
    None
}

fn output_says_logged_out(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "not logged in",
        "not authenticated",
        "unauthenticated",
        "no account",
        "please log in",
        "auth login",
    ]
    .iter()
    .any(|n| lower.contains(n))
}

// ---- reads -----------------------------------------------------------------------------

/// Read-only Jira access through `acli`. Holds no state besides the runner; constructing it
/// spawns nothing.
pub struct AcliJira<R = ProcessRunner> {
    inner: Acli<R>,
    /// Notes for the caller (a truncated comment list); drained by `take_warnings`.
    warnings: Mutex<Vec<String>>,
}

impl AcliJira<ProcessRunner> {
    pub fn new(config: &Config) -> ProviderResult<Self> {
        Ok(Self::with_runner(
            ProcessRunner::new().with_timeout(ACLI_TIMEOUT),
            site_of(config),
        ))
    }
}

impl<R: CommandRunner + Send + Sync> AcliJira<R> {
    /// An adapter over a custom runner (tests use a fake).
    pub fn with_runner(runner: R, site: Option<String>) -> Self {
        Self {
            inner: Acli { runner, site },
            warnings: Mutex::new(Vec::new()),
        }
    }
}

impl<R: CommandRunner + Send + Sync> TicketProvider for AcliJira<R> {
    fn health(&self) -> Health {
        let version_out = match self.inner.run_raw(&["--version"]) {
            Ok(o) => o,
            Err(ProviderError::NotInstalled { detail, .. }) => {
                return Health::not_installed(format!(
                    "{detail}; install Atlassian's `acli` (https://developer.atlassian.com/cloud/acli/guides/install-acli/)"
                ));
            }
            Err(e) => {
                return Health {
                    installed: false,
                    version: None,
                    authenticated: false,
                    detail: format!("could not run acli: {e}"),
                    meets_minimum: false,
                };
            }
        };
        if !version_out.success {
            return Health {
                installed: true,
                version: None,
                authenticated: false,
                detail: format!(
                    "`acli --version` failed: {}",
                    snippet(&version_out.stderr, 200)
                ),
                meets_minimum: false,
            };
        }
        let raw = format!("{}\n{}", version_out.stdout, version_out.stderr);
        let parsed = parse_version(&raw);
        let min = parse_version(MIN_ACLI_VERSION).unwrap_or((0, 0, 0));
        let meets_minimum = parsed.is_some_and(|v| v >= min);
        let version = parsed.map(|(a, b, c)| format!("{a}.{b}.{c}"));

        let (authenticated, auth_detail) = match self.inner.run_raw(&["jira", "auth", "status"]) {
            Ok(o) if o.success && !output_says_logged_out(&format!("{}{}", o.stdout, o.stderr)) => {
                (true, String::new())
            }
            Ok(o) => (
                false,
                format!(
                    "not logged in to Jira (run `acli jira auth login`): {}",
                    snippet(
                        if o.stderr.trim().is_empty() {
                            &o.stdout
                        } else {
                            &o.stderr
                        },
                        200
                    )
                ),
            ),
            Err(e) => (false, format!("could not check login: {e}")),
        };

        let detail = if !authenticated {
            auth_detail
        } else if parsed.is_none() {
            format!("could not read the version from {:?}", snippet(&raw, 100))
        } else if !meets_minimum {
            format!("acli is older than {MIN_ACLI_VERSION}; upgrade it")
        } else {
            "ready".into()
        };
        Health {
            installed: true,
            version,
            authenticated,
            detail,
            meets_minimum,
        }
    }

    fn search(&self, jql: &str) -> ProviderResult<Vec<RemoteTicket>> {
        // `--paginate` makes acli fetch every page ("Fetch all work items by paginating
        // through the results"); the parser also accepts one JSON document per page.
        let out = self.inner.run(&[
            "jira",
            "workitem",
            "search",
            "--jql",
            jql,
            "--paginate",
            "--json",
            "--fields",
            SEARCH_FIELDS,
        ])?;
        wire::parse_search(&out.stdout, self.inner.site.as_deref())
    }

    fn get(&self, key: &TicketKey) -> ProviderResult<RemoteTicket> {
        let out = self.inner.run(&[
            "jira",
            "workitem",
            "view",
            key.as_str(),
            "--json",
            "--fields",
            VIEW_FIELDS,
        ])?;
        wire::parse_view(&out.stdout, self.inner.site.as_deref())?
            .ok_or_else(|| ProviderError::NotFound(format!("{key} was not returned by acli")))
    }

    fn comments(&self, key: &TicketKey) -> ProviderResult<Vec<RemoteComment>> {
        let out = self.inner.run(&[
            "jira",
            "workitem",
            "view",
            key.as_str(),
            "--json",
            "--fields",
            COMMENT_FIELDS,
        ])?;
        let (comments, total) = wire::parse_comment_field(&out.stdout, key)?;
        if let Some(total) = total
            && total > comments.len()
        {
            self.warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!(
                    "{key}: acli returned {} of {total} comments; some are missing",
                    comments.len()
                ));
        }
        Ok(comments)
    }

    fn take_warnings(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

// ---- writes ----------------------------------------------------------------------------

/// Write access through `acli`. **Only the write gateway constructs one.**
pub struct AcliJiraWriter<R = ProcessRunner> {
    inner: Acli<R>,
}

impl AcliJiraWriter<ProcessRunner> {
    /// `pub(crate)`: code outside `de-core` (the CLI, a GUI) cannot build a writer at all.
    pub(crate) fn new(config: &Config) -> ProviderResult<Self> {
        Ok(Self::with_runner(
            ProcessRunner::new().with_timeout(ACLI_TIMEOUT),
            site_of(config),
        ))
    }
}

impl<R: CommandRunner + Send + Sync> AcliJiraWriter<R> {
    pub(crate) fn with_runner(runner: R, site: Option<String>) -> Self {
        Self {
            inner: Acli { runner, site },
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

impl<R: CommandRunner + Send + Sync> TicketWriter for AcliJiraWriter<R> {
    /// Posts `body` exactly as given, as one argv element (no shell, no decoration).
    ///
    /// `--body` accepts "plain text or ADF", so a body that is itself an ADF JSON document
    /// would be posted as formatted content rather than verbatim; such a body is refused.
    fn add_comment(&self, key: &TicketKey, body: &str) -> ProviderResult<RemoteComment> {
        if body.trim().is_empty() {
            return Err(ProviderError::Unsupported(
                "refusing to post an empty comment".into(),
            ));
        }
        if let Ok(doc) = serde_json::from_str::<Value>(body.trim())
            && doc.get("type").and_then(Value::as_str) == Some("doc")
        {
            return Err(ProviderError::Unsupported(
                "the comment is an ADF JSON document, which acli would interpret instead of posting verbatim".into(),
            ));
        }
        let out = self.inner.run(&[
            "jira",
            "workitem",
            "comment",
            "create",
            "--key",
            key.as_str(),
            "--body",
            body,
            "--json",
        ])?;
        // The comment exists now: never fail (and invite a duplicate retry) just because the
        // reply is unreadable. UNVERIFIED that `create --json` prints the comment.
        match wire::parse_created_comment(&out.stdout, key) {
            Ok(Some(comment)) => Ok(comment),
            Ok(None) | Err(_) => {
                let (_, mentions) = body_to_text(&Value::String(body.into()));
                Ok(RemoteComment {
                    id: String::new(),
                    ticket: key.clone(),
                    author_account_id: String::new(),
                    author_name: String::new(),
                    body_text: body.into(),
                    mentions,
                    created_at: now_secs(),
                })
            }
        }
    }

    /// Moves the ticket to the status named `to_status`, via `acli ... transition --status`.
    ///
    /// `acli` documents no way to list a work item's available transitions, so a status that
    /// cannot be reached surfaces as [`ProviderError::Unsupported`] carrying acli's own
    /// message (which may name the valid statuses); this adapter never guesses ids.
    fn transition(&self, key: &TicketKey, to_status: &str) -> ProviderResult<()> {
        if to_status.trim().is_empty() {
            return Err(ProviderError::Unsupported("no target status given".into()));
        }
        let output = self.inner.run_raw(&[
            "jira",
            "workitem",
            "transition",
            "--key",
            key.as_str(),
            "--status",
            to_status,
            "--yes",
            "--json",
        ])?;
        let unreachable = |detail: &str| {
            ProviderError::Unsupported(format!(
                "cannot move {key} to {to_status:?}: {detail}. acli cannot list the available transitions; check the workflow in Jira"
            ))
        };
        if !output.success {
            return Err(match classify_failure(&output) {
                e @ (ProviderError::NotInstalled { .. }
                | ProviderError::NotAuthenticated { .. }
                | ProviderError::Network(_)) => e,
                other => {
                    let detail = match other {
                        ProviderError::Command { stderr, .. } => stderr,
                        e => e.to_string(),
                    };
                    unreachable(&detail)
                }
            });
        }
        // UNVERIFIED: with `--json` acli may report per-item failures with exit code 0.
        if let Ok(doc) = serde_json::from_str::<Value>(output.stdout.trim())
            && reports_failure(&doc)
        {
            return Err(unreachable(&snippet(&output.stdout, 300)));
        }
        Ok(())
    }
}

fn reports_failure(doc: &Value) -> bool {
    match doc {
        Value::Object(o) => {
            o.get("success") == Some(&Value::Bool(false)) || o.values().any(reports_failure)
        }
        Value::Array(a) => a.iter().any(reports_failure),
        _ => false,
    }
}

// ---- probe -----------------------------------------------------------------------------

/// The raw result of one harmless command, for verifying the parsers on a real install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// The command line as run.
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs only read-only commands (version, auth status, a one-result search, and, for
/// `sample_key`, view and view of the comment field) and returns their raw output so it can be pasted
/// into a bug report or fixture. Never fails: a command that cannot start is a result with
/// `exit_code: None` and the reason in `stderr`. No comment or transition command is here.
pub fn probe(runner: &dyn CommandRunner, sample_key: Option<&TicketKey>) -> Vec<ProbeResult> {
    let mut commands: Vec<Vec<String>> = vec![
        vec!["--version".into()],
        vec!["jira".into(), "auth".into(), "status".into()],
        [
            "jira",
            "workitem",
            "search",
            "--jql",
            // Unbounded JQL (only an `order by`) is rejected by acli.
            "assignee = currentUser() order by updated DESC",
            "--limit",
            "1",
            "--json",
            "--fields",
            SEARCH_FIELDS,
        ]
        .map(String::from)
        .to_vec(),
    ];
    if let Some(key) = sample_key {
        let key = key.as_str();
        commands.push(
            [
                "jira",
                "workitem",
                "view",
                key,
                "--json",
                "--fields",
                VIEW_FIELDS,
            ]
            .map(String::from)
            .to_vec(),
        );
        commands.push(
            [
                "jira",
                "workitem",
                "view",
                key,
                "--json",
                "--fields",
                COMMENT_FIELDS,
            ]
            .map(String::from)
            .to_vec(),
        );
    }
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    commands
        .into_iter()
        .map(|args| {
            let shown = std::iter::once(PROGRAM.to_string())
                .chain(args.iter().map(|a| {
                    if a.contains(' ') {
                        format!("{a:?}")
                    } else {
                        a.clone()
                    }
                }))
                .collect::<Vec<_>>()
                .join(" ");
            let external = ExternalCommand {
                dir: dir.clone(),
                program: PROGRAM.into(),
                args,
                label: shown.clone(),
            };
            match runner.run(&external) {
                Ok(o) => ProbeResult {
                    command: shown,
                    exit_code: o.code,
                    stdout: o.stdout,
                    stderr: o.stderr,
                },
                Err(e) => ProbeResult {
                    command: shown,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("could not run: {e:#}"),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;

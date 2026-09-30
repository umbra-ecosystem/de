//! Jira adapter over Atlassian's `acli` (`acli jira workitem ...`).
//!
//! [`AcliJira`] implements [`TicketProvider`] (reads only) and [`AcliJiraWriter`]
//! implements [`TicketWriter`] (writes only); the registry builds them separately.
//! Both run `acli` through a [`CommandRunner`] (a [`ProcessRunner`] in production) and never
//! handle credentials: `acli` owns its login.
//!
//! # Commands used, and where each comes from
//!
//! Documentation: <https://developer.atlassian.com/cloud/acli/reference/commands/>.
//! **VERIFIED** means the command and flag are in that reference; **UNVERIFIED** means they
//! are assumed. Nothing here was run against a real `acli` (none was available).
//!
//! | Purpose | Invocation | Status |
//! |---|---|---|
//! | version | `acli --version` | UNVERIFIED (the docs never show a version command; cobra convention) |
//! | auth check | `acli jira auth status` | VERIFIED (`jira-auth-status`); output and exit code when logged out UNVERIFIED |
//! | search | `acli jira workitem search --jql Q --paginate --json --fields ...` | VERIFIED flags (`--jql`, `--paginate`, `--json`, `--fields`, `--limit`) |
//! | view | `acli jira workitem view KEY --json --fields ...` | VERIFIED |
//! | comments | `acli jira workitem comment list --key KEY --paginate --json` | VERIFIED (`comment list`: `--key`, `--json`, `--paginate`, `--limit`, `--order`) |
//! | add comment | `acli jira workitem comment create --key KEY --body TEXT --json` | VERIFIED flags (`--key`, `--body`, `--json`); the JSON it prints UNVERIFIED |
//! | transition | `acli jira workitem transition --key KEY --status NAME --yes --json` | VERIFIED (`-k/--key`, `-s/--status`, `-y/--yes`, `--json`) |
//!
//! The JSON shapes of `view`, `search` and `comment list` are not documented at all; see
//! [`wire`]. `--fields` values for search/view are documented (comma-separated field names);
//! that `updated` is a valid field name is standard Jira. `acli` has **no** command to list
//! the transitions available for a work item, so [`AcliJiraWriter::transition`] cannot
//! enumerate them (it reports acli's own error instead).
//!
//! `acli` 1.3.4 (19 September 2025) is the release that introduced `comment list`
//! (<https://developer.atlassian.com/cloud/acli/changelog/>), which sync cannot work
//! without, hence [`MIN_ACLI_VERSION`].

mod adf;
mod time;
mod wire;

use std::io;
use std::path::PathBuf;

use serde_json::Value;

use super::error::{ProviderError, ProviderResult};
use super::model::{Health, RemoteComment, RemoteTicket};
use super::traits::{TicketProvider, TicketWriter};
use crate::config::Config;
use crate::domain::TicketKey;
use crate::overlay::{CommandOutput, CommandRunner, ExternalCommand, ProcessRunner};

pub use adf::{adf_to_text, body_to_text};
pub use time::parse_timestamp;
use wire::{TOOL, snippet};

/// The oldest `acli` this adapter is written for: 1.3.4, the first release with
/// `acli jira workitem comment list` (changelog entry of 19 September 2025). Older versions
/// lack the command entirely, so they are reported as not meeting the minimum.
pub const MIN_ACLI_VERSION: &str = "1.3.4";

const PROGRAM: &str = "acli";

/// Fields requested from search and view. UNVERIFIED that every name is accepted by
/// `--fields`, but they are plain Jira field ids.
const TICKET_FIELDS: &str = "key,summary,status,priority,assignee,updated";

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
        if output.success {
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

fn spawn_error(e: &eyre::Report) -> ProviderError {
    if is_not_found(e) {
        ProviderError::not_installed(TOOL, "`acli` was not found on PATH")
    } else {
        ProviderError::Command {
            tool: TOOL.into(),
            status: None,
            stderr: format!("{e:#}"),
        }
    }
}

fn has_token(text: &str, token: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| t == token)
}

/// Turns a failed `acli` run into the error sync can act on. Matches on wording because
/// `acli` documents no exit codes (UNVERIFIED): auth first, then network, then not found.
fn classify_failure(output: &CommandOutput) -> ProviderError {
    let detail = if output.stderr.trim().is_empty() {
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
    ]) || has_token(&lower, "401")
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
    } else if (any(&["does not exist", "not found"]) && !any(&["command not found"]))
        || has_token(&lower, "404")
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
}

impl AcliJira<ProcessRunner> {
    pub fn new(config: &Config) -> ProviderResult<Self> {
        Ok(Self::with_runner(ProcessRunner::new(), site_of(config)))
    }
}

impl<R: CommandRunner + Send + Sync> AcliJira<R> {
    /// An adapter over a custom runner (tests use a fake).
    pub fn with_runner(runner: R, site: Option<String>) -> Self {
        Self {
            inner: Acli { runner, site },
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
            TICKET_FIELDS,
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
            TICKET_FIELDS,
        ])?;
        wire::parse_view(&out.stdout, self.inner.site.as_deref())?
            .ok_or_else(|| ProviderError::NotFound(format!("{key} was not returned by acli")))
    }

    fn comments(&self, key: &TicketKey) -> ProviderResult<Vec<RemoteComment>> {
        let out = self.inner.run(&[
            "jira",
            "workitem",
            "comment",
            "list",
            "--key",
            key.as_str(),
            "--paginate",
            "--json",
        ])?;
        wire::parse_comments(&out.stdout, key)
    }
}

// ---- writes ----------------------------------------------------------------------------

/// Write access through `acli`. **Only the write gateway constructs one.**
pub struct AcliJiraWriter<R = ProcessRunner> {
    inner: Acli<R>,
}

impl AcliJiraWriter<ProcessRunner> {
    pub fn new(config: &Config) -> ProviderResult<Self> {
        Ok(Self::with_runner(ProcessRunner::new(), site_of(config)))
    }
}

impl<R: CommandRunner + Send + Sync> AcliJiraWriter<R> {
    pub fn with_runner(runner: R, site: Option<String>) -> Self {
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
/// `sample_key`, view and a comment list) and returns their raw output so it can be pasted
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
            "order by updated DESC",
            "--limit",
            "1",
            "--json",
            "--fields",
            TICKET_FIELDS,
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
                TICKET_FIELDS,
            ]
            .map(String::from)
            .to_vec(),
        );
        commands.push(
            [
                "jira", "workitem", "comment", "list", "--key", key, "--limit", "5", "--json",
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

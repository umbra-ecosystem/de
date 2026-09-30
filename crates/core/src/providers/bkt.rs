//! Bitbucket adapter over the community `bkt` CLI (avivsinai/bitbucket-cli, Go, MIT).
//!
//! **Only a little of this was run against a real `bkt` (0.32.1, not logged in).** Verified
//! for real: `bkt --version` prints `bkt version 0.32.1`; `bkt auth status --json` when logged
//! out exits 0 with `{"hosts": null, "contexts": null}`; every command that needs a context
//! exits 1 with ``Error: no active context; run `bkt context use <name>` ``. Everything else
//! was written from the tool's source (master, v0.32.1, `pkg/cmd/*`, `pkg/bbcloud/*`) and
//! README; "VERIFIED" below means read in that source, `UNVERIFIED:` means assumed. Use
//! [`probe`] on a logged-in install and paste the output to confirm or repair the parsers.
//!
//! # Cloud only
//! `bkt pipeline *` is Cloud-only in the source ("commands are no-ops for Data Center"), and
//! DC's PR JSON differs (`fromRef`/`toRef`, `reviewers[].approved`). Data Center is therefore
//! [`ProviderError::Unsupported`] for every call. The kind is read once from
//! `bkt auth status --json` (local config, no network).
//!
//! # What `bkt` can and cannot do (all VERIFIED from source unless noted)
//! - `bkt pr list --workspace W --repo S --state OPEN|MERGED|DECLINED|ALL --limit N --json`
//!   (`--limit 0` = all pages, follows `next`; output `{"pull_requests":[...]}`). Cloud list
//!   entries may carry partial `participants`.
//! - `bkt pr view ID --workspace W --repo S --json` -> `{"pull_request":{..}}` including
//!   `destination.branch.name`, `reviewers[]` and `participants[]` (`approved`, `state`
//!   `approved`/`changes_requested`).
//! - `bkt pr comments ID --workspace W --repo S --json` -> `{"comments":[..]}` (all pages;
//!   `inline{path,from,to}`; `to` = new side, `from` = old side).
//! - `bkt pr comment ID --text T --file F --to-line N|--from-line N` creates inline comments
//!   but prints only a text line, not the created comment. The adapter uses the raw API
//!   instead so it gets the created comment (id) back.
//! - `bkt pr approve ID --workspace W --repo S`.
//! - No `request-changes` and no pipeline `rerun` command exist.
//! - `bkt pipeline list/view --json` **drops the commit hash and the deployment environment**
//!   (its `Pipeline`/`PipelineStep` structs only keep uuid/build_number/state/ref name/times
//!   and step uuid/name/state), and has no commit filter. So pipelines go through the raw
//!   passthrough.
//! - `bkt api PATH [--method M] [--param k=v]... [--input JSON]` is a `gh api` equivalent
//!   (VERIFIED, `pkg/cmd/api/api.go`): relative paths are joined to the Cloud base
//!   `https://api.bitbucket.org/2.0`, the response body is written to stdout, non-2xx exits
//!   non-zero. `--input` takes the JSON body as an argument (no shell involved).
//! - `bkt --version` (cobra: `bkt version X.Y.Z`; `dev` for source builds).
//!
//! # Pipeline by commit
//! Neither the CLI nor (verified) the adapter can filter server side by commit:
//! [`CodeHost::pipelines`] lists newest-first through the raw REST endpoint (pagelen 100) and
//! filters `target.commit.hash` with [`commit_matches`] in the adapter, scanning at most
//! [`MAX_SCAN_PAGES`] pages (500 runs). A run older than that is not found. Whether the REST
//! list also accepts a `target.commit.hash` query filter is UNVERIFIED and not relied on.
//!
//! # Deployment environment (the point of "deployed")
//! UNVERIFIED: the REST `pipeline_step` object is read for `environment` (object with
//! `name`/`slug`/`uuid`, or a string) or `deployment.environment`. A bare uuid is resolved to
//! a name via `GET /repositories/W/S/environments` (VERIFIED endpoint, used by `bkt variable`).
//! If none of these are present the step has no environment and the run never counts as
//! deployed; fix from probe output.

mod probe;
mod runner;
mod wire;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::error::{ProviderError, ProviderResult};
use super::model::{
    Health, NewPrComment, PipelineFilter, PipelineRun, Pr, PrComment, PrFilter, PrState,
    TriggerSpec, commit_matches,
};
use super::traits::{CodeHost, CodeHostWriter};
use crate::config::Config;
use wire::{
    EnvRef, WAuthStatus, WComment, WEnvironment, WPage, WPipeline, WPr, WStep, parse_version,
    snippet,
};

pub use probe::{ProbeResult, probe};
pub use runner::{BktRunner, RawOutput, RunError, SystemRunner};

const TOOL: &str = "bkt";

/// Oldest `bkt` this adapter is written against. The README pins 0.26.0 in its headless
/// example; everything used (`pr list/view/comments/approve`, `api`, `auth status --json`,
/// `--json`, inline comment flags since 0.14) predates it per CHANGELOG.md. UNVERIFIED that
/// it is the true minimum; raise it if a real install shows otherwise.
pub const MIN_BKT_VERSION: &str = "0.26.0";

/// Pages of 100 runs scanned when filtering pipelines by commit or branch.
pub const MAX_SCAN_PAGES: usize = 5;

const DEFAULT_PIPELINE_LIMIT: usize = 30;
const MAX_PAGES: usize = 20;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn args<const N: usize>(a: [&str; N]) -> Vec<String> {
    a.into_iter().map(String::from).collect()
}

fn split_repo(repo: &str) -> ProviderResult<(&str, &str)> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
    };
    match repo.split_once('/') {
        Some((w, s)) if ok(w) && ok(s) => Ok((w, s)),
        _ => Err(ProviderError::Other(format!(
            "invalid repo {repo:?}: expected `workspace/slug`"
        ))),
    }
}

/// A pipeline id: build number or `{uuid}`; braces are percent-encoded for the path.
fn encode_pipeline_id(id: &str) -> ProviderResult<String> {
    let id = id.trim();
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '{' | '}' | '-'));
    if !ok {
        return Err(ProviderError::Other(format!("invalid pipeline id {id:?}")));
    }
    Ok(id.replace('{', "%7B").replace('}', "%7D"))
}

/// Whether `lower` names the HTTP status `code` as a status, in one of the ways Go HTTP
/// clients print it. A bare number is not enough: PR ids, ports and pipeline numbers are
/// numbers too (`pull request 401` is not an authentication failure).
fn has_http_status(lower: &str, code: &str) -> bool {
    [
        format!("status {code}"),
        format!("status: {code}"),
        format!("status code {code}"),
        format!("http {code}"),
        format!("http/1.1 {code}"),
        format!("error {code}"),
        format!("({code})"),
        format!("[{code}]"),
        format!("{code} unauthorized"),
        format!("{code} not found"),
    ]
    .iter()
    .any(|needle| lower.contains(needle.as_str()))
}

/// Maps a failed `bkt` run onto a typed error. The stderr wording is from `bkt`'s source
/// (`pkg/cmdutil/context.go`, `pkg/httpx/client.go`); matching is loose. Only the
/// `no active context` wording is verified against a real bkt 0.32.1.
fn classify(out: &RawOutput) -> ProviderError {
    let text = format!("{} {}", out.stderr, out.stdout);
    let lower = text.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if has(&["cloud contexts only"]) {
        return ProviderError::Unsupported(
            "bkt is pointed at a Bitbucket Data Center context; only Cloud is supported".into(),
        );
    }
    if has(&["no active context"]) {
        // Verified against bkt 0.32.1: every command needing a context exits 1 with
        // "Error: no active context; run `bkt context use <name>`".
        return ProviderError::not_authenticated(
            TOOL,
            format!(
                "{}; log in with `bkt auth login https://bitbucket.org --kind cloud --web`, then select a context with `bkt context use <name>`",
                snippet(&out.stderr)
            ),
        );
    }
    if has(&[
        "no hosts configured",
        "auth login",
        "credentials for host",
        "unauthorized",
        "invalid credentials",
    ]) || has_http_status(&lower, "401")
    {
        return ProviderError::not_authenticated(TOOL, snippet(&out.stderr));
    }
    if has(&[
        "no such host",
        "dial tcp",
        "connection refused",
        "i/o timeout",
        "network is unreachable",
        "tls:",
        "timed out",
    ]) {
        return ProviderError::Network(snippet(&out.stderr));
    }
    if has(&["not found"]) || has_http_status(&lower, "404") {
        return ProviderError::NotFound(snippet(&out.stderr));
    }
    ProviderError::Command {
        tool: TOOL.into(),
        status: out.code,
        stderr: snippet(&out.stderr),
    }
}

struct Client<R> {
    runner: R,
    cloud_ok: Mutex<bool>,
    envs: Mutex<HashMap<String, Vec<WEnvironment>>>,
}

impl<R: BktRunner> Client<R> {
    fn new(runner: R) -> Self {
        Self {
            runner,
            cloud_ok: Mutex::new(false),
            envs: Mutex::new(HashMap::new()),
        }
    }

    fn exec(&self, argv: Vec<String>) -> ProviderResult<RawOutput> {
        match self.runner.run(&argv) {
            Err(RunError::NotFound) => Err(ProviderError::not_installed(
                TOOL,
                "`bkt` was not found on PATH (brew install avivsinai/tap/bitbucket-cli)",
            )),
            Err(RunError::Failed(m)) => {
                if m.contains("timed out") {
                    Err(ProviderError::Network(m))
                } else {
                    Err(ProviderError::Command {
                        tool: TOOL.into(),
                        status: None,
                        stderr: m,
                    })
                }
            }
            Ok(o) if o.success() => Ok(o),
            Ok(o) => Err(classify(&o)),
        }
    }

    fn parse<T: DeserializeOwned>(what: &str, text: &str) -> ProviderResult<T> {
        serde_json::from_str(text).map_err(|e| ProviderError::Parse {
            tool: TOOL.into(),
            what: what.into(),
            detail: format!("{e}; output began: {}", snippet(text)),
        })
    }

    fn convert<T>(what: &str, text: &str, r: Result<T, String>) -> ProviderResult<T> {
        r.map_err(|e| ProviderError::Parse {
            tool: TOOL.into(),
            what: what.into(),
            detail: format!("{e}; output began: {}", snippet(text)),
        })
    }

    /// Local, no network: reads `bkt auth status --json` and refuses Data Center.
    fn ensure_cloud(&self) -> ProviderResult<()> {
        if *lock(&self.cloud_ok) {
            return Ok(());
        }
        let out = self.exec(args(["auth", "status", "--json"]))?;
        let status: WAuthStatus = Self::parse("auth status", &out.stdout)?;
        Self::detect_kind(&status)?;
        *lock(&self.cloud_ok) = true;
        Ok(())
    }

    fn detect_kind(status: &WAuthStatus) -> ProviderResult<&'static str> {
        if status.hosts.is_empty() {
            return Err(ProviderError::not_authenticated(
                TOOL,
                "no hosts configured (`bkt auth status` reports null hosts and contexts when logged out); \
                 run `bkt auth login https://bitbucket.org --kind cloud --web`, then `bkt context use <name>`",
            ));
        }
        let active_kind = status
            .contexts
            .iter()
            .find(|c| c.active)
            .and_then(|c| status.hosts.iter().find(|h| h.key == c.host))
            .map(|h| h.kind.as_str());
        let cloud = match active_kind {
            Some(kind) => kind == "cloud",
            None => status.hosts.iter().any(|h| h.kind == "cloud"),
        };
        if cloud {
            Ok("cloud")
        } else {
            Err(ProviderError::Unsupported(
                "Bitbucket Data Center is not supported yet (Cloud only): `bkt pipeline` is \
                 Cloud-only and Data Center output differs"
                    .into(),
            ))
        }
    }

    fn health(&self) -> Health {
        let version_out = match self.runner.run(&args(["--version"])) {
            Err(RunError::NotFound) => {
                return Health::not_installed(
                    "`bkt` was not found on PATH (brew install avivsinai/tap/bitbucket-cli)",
                );
            }
            Err(RunError::Failed(m)) => return Health::not_installed(m),
            Ok(o) => o,
        };
        let mut h = Health {
            installed: true,
            version: None,
            authenticated: false,
            detail: String::new(),
            meets_minimum: false,
        };
        if !version_out.success() {
            h.detail = format!("`bkt --version` failed: {}", snippet(&version_out.stderr));
            return h;
        }
        let text = format!("{}{}", version_out.stdout, version_out.stderr);
        match parse_version(&text) {
            Some(v) => {
                h.version = Some(format!("{}.{}.{}", v.0, v.1, v.2));
                h.meets_minimum = parse_version(MIN_BKT_VERSION).is_some_and(|min| v >= min);
                if !h.meets_minimum {
                    h.detail = format!(
                        "bkt {}.{}.{} is older than {MIN_BKT_VERSION}",
                        v.0, v.1, v.2
                    );
                    return h;
                }
            }
            None => {
                // Source builds print `dev`; we cannot compare, so do not block on it.
                h.version = Some(snippet(text.lines().next().unwrap_or("")));
                h.meets_minimum = true;
            }
        }
        match self.runner.run(&args(["auth", "status", "--json"])) {
            Err(e) => h.detail = format!("could not run `bkt auth status`: {e:?}"),
            Ok(o) if !o.success() => {
                h.detail = format!("`bkt auth status` failed: {}", snippet(&o.stderr));
            }
            Ok(o) => match serde_json::from_str::<WAuthStatus>(&o.stdout) {
                Err(e) => h.detail = format!("could not parse `bkt auth status --json`: {e}"),
                Ok(st) => match Self::detect_kind(&st) {
                    Ok(_) => {
                        h.authenticated = true;
                        h.detail = "ready (Bitbucket Cloud; login is configured, token not \
                                    checked against the network)"
                            .into();
                    }
                    Err(e) => h.detail = e.to_string(),
                },
            },
        }
        h
    }

    // ---- reads ----

    fn list_prs(&self, repo: &str, filter: &PrFilter) -> ProviderResult<Vec<Pr>> {
        let (w, s) = split_repo(repo)?;
        self.ensure_cloud()?;
        let states: Vec<PrState> = match filter.state {
            Some(st) => vec![st],
            // `--state ALL` exists in source but skips SUPERSEDED, so ask per state.
            None => PrState::ALL.to_vec(),
        };
        let mut out: Vec<Pr> = Vec::new();
        for st in states {
            let up = st.as_str().to_ascii_uppercase();
            let o = self.exec(args([
                "pr",
                "list",
                "--workspace",
                w,
                "--repo",
                s,
                "--state",
                &up,
                "--limit",
                "0",
                "--json",
            ]))?;
            let v: Value = Self::parse("pull request list", &o.stdout)?;
            let items = match &v {
                Value::Array(a) => a.clone(),
                other => other
                    .get("pull_requests")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| ProviderError::Parse {
                        tool: TOOL.into(),
                        what: "pull request list".into(),
                        detail: format!(
                            "no `pull_requests` array; output began: {}",
                            snippet(&o.stdout)
                        ),
                    })?,
            };
            for item in items {
                let wp: WPr = serde_json::from_value(item).map_err(|e| ProviderError::Parse {
                    tool: TOOL.into(),
                    what: "pull request".into(),
                    detail: format!("{e}; output began: {}", snippet(&o.stdout)),
                })?;
                let pr = Self::convert("pull request", &o.stdout, wp.into_pr(repo))?;
                if !out.iter().any(|p| p.id == pr.id) {
                    out.push(pr);
                }
            }
        }
        if let Some(text) = filter.text.as_deref().map(str::to_ascii_lowercase) {
            out.retain(|p| {
                p.title.to_ascii_lowercase().contains(&text)
                    || p.source_branch.to_ascii_lowercase().contains(&text)
            });
        }
        Ok(out)
    }

    fn pr(&self, repo: &str, id: u64) -> ProviderResult<Pr> {
        let (w, s) = split_repo(repo)?;
        self.ensure_cloud()?;
        let o = self.exec(args([
            "pr",
            "view",
            &id.to_string(),
            "--workspace",
            w,
            "--repo",
            s,
            "--json",
        ]))?;
        let v: Value = Self::parse("pull request", &o.stdout)?;
        let inner = v.get("pull_request").cloned().unwrap_or(v);
        let wp: WPr = serde_json::from_value(inner).map_err(|e| ProviderError::Parse {
            tool: TOOL.into(),
            what: "pull request".into(),
            detail: format!("{e}; output began: {}", snippet(&o.stdout)),
        })?;
        Self::convert("pull request", &o.stdout, wp.into_pr(repo))
    }

    fn pr_comments(&self, repo: &str, id: u64) -> ProviderResult<Vec<PrComment>> {
        let (w, s) = split_repo(repo)?;
        self.ensure_cloud()?;
        let o = self.exec(args([
            "pr",
            "comments",
            &id.to_string(),
            "--workspace",
            w,
            "--repo",
            s,
            "--json",
        ]))?;
        let v: Value = Self::parse("pull request comments", &o.stdout)?;
        let items = match &v {
            Value::Array(a) => a.clone(),
            other => other
                .get("comments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        };
        let mut out = Vec::new();
        for item in items {
            let wc: WComment = serde_json::from_value(item).map_err(|e| ProviderError::Parse {
                tool: TOOL.into(),
                what: "pull request comment".into(),
                detail: format!("{e}; output began: {}", snippet(&o.stdout)),
            })?;
            if wc.deleted {
                continue;
            }
            out.push(Self::convert(
                "pull request comment",
                &o.stdout,
                wc.into_comment(id),
            )?);
        }
        out.sort_by_key(|c| (c.created_at, c.id));
        Ok(out)
    }

    /// `bkt api PATH --param k=v...` (GET; never passes `--method`).
    fn api_get(&self, path: &str, params: &[(&str, String)]) -> ProviderResult<RawOutput> {
        let mut argv = args(["api", path]);
        for (k, v) in params {
            argv.push("--param".into());
            argv.push(format!("{k}={v}"));
        }
        self.exec(argv)
    }

    fn pipelines(&self, repo: &str, filter: &PipelineFilter) -> ProviderResult<Vec<PipelineRun>> {
        let (w, s) = split_repo(repo)?;
        self.ensure_cloud()?;
        let limit = filter.limit.unwrap_or(DEFAULT_PIPELINE_LIMIT).max(1);
        let filtered = filter.branch.is_some() || filter.commit.is_some();
        let pagelen = if filtered { 100 } else { limit.min(100) };
        let max_pages = if filtered { MAX_SCAN_PAGES } else { MAX_PAGES };
        let path = format!("/repositories/{w}/{s}/pipelines/");
        let mut out: Vec<PipelineRun> = Vec::new();
        for page in 1..=max_pages {
            let mut params = vec![
                ("pagelen", pagelen.to_string()),
                ("sort", "-created_on".into()),
            ];
            if page > 1 {
                params.push(("page", page.to_string()));
            }
            let o = self.api_get(&path, &params)?;
            let p: WPage<WPipeline> = Self::parse("pipeline list", &o.stdout)?;
            for wp in p.values {
                let run = Self::convert("pipeline", &o.stdout, wp.into_run(repo))?;
                if let Some(b) = &filter.branch
                    && run.branch != *b
                {
                    continue;
                }
                if let Some(c) = &filter.commit
                    && (run.commit.is_empty() || !commit_matches(&run.commit, c))
                {
                    continue;
                }
                out.push(run);
                if out.len() >= limit {
                    return Ok(out);
                }
            }
            if p.next.is_empty() {
                break;
            }
        }
        Ok(out)
    }

    fn fetch_run(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        let (w, s) = split_repo(repo)?;
        let enc = encode_pipeline_id(id)?;
        let o = self.api_get(&format!("/repositories/{w}/{s}/pipelines/{enc}"), &[])?;
        let wp: WPipeline = Self::parse("pipeline", &o.stdout)?;
        Self::convert("pipeline", &o.stdout, wp.into_run(repo))
    }

    fn pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        self.ensure_cloud()?;
        let mut run = self.fetch_run(repo, id)?;
        let (w, s) = split_repo(repo)?;
        let enc = encode_pipeline_id(&run.id)?;
        let path = format!("/repositories/{w}/{s}/pipelines/{enc}/steps/");
        for page in 1..=MAX_PAGES {
            let mut params = vec![("pagelen", "100".to_string())];
            if page > 1 {
                params.push(("page", page.to_string()));
            }
            let o = self.api_get(&path, &params)?;
            let p: WPage<WStep> = Self::parse("pipeline steps", &o.stdout)?;
            for step in p.values {
                let env = match step.env_ref() {
                    Some(r) => Some(self.env_name(repo, &r)?),
                    None => None,
                };
                run.steps.push(step.into_step(env));
            }
            if p.next.is_empty() {
                break;
            }
        }
        Ok(run)
    }

    fn env_name(&self, repo: &str, r: &EnvRef) -> ProviderResult<String> {
        if !r.name.is_empty() {
            return Ok(r.name.clone());
        }
        if !r.uuid.is_empty() {
            if !lock(&self.envs).contains_key(repo) {
                let (w, s) = split_repo(repo)?;
                let mut all = Vec::new();
                for page in 1..=MAX_PAGES {
                    let mut params = vec![("pagelen", "100".to_string())];
                    if page > 1 {
                        params.push(("page", page.to_string()));
                    }
                    let o =
                        self.api_get(&format!("/repositories/{w}/{s}/environments"), &params)?;
                    let p: WPage<WEnvironment> = Self::parse("deployment environments", &o.stdout)?;
                    all.extend(p.values);
                    if p.next.is_empty() {
                        break;
                    }
                }
                lock(&self.envs).insert(repo.into(), all);
            }
            let envs = lock(&self.envs);
            if let Some(e) = envs
                .get(repo)
                .and_then(|l| l.iter().find(|e| e.uuid.eq_ignore_ascii_case(&r.uuid)))
            {
                let name = if e.name.is_empty() { &e.slug } else { &e.name };
                if !name.is_empty() {
                    return Ok(name.clone());
                }
            }
            if r.slug.is_empty() {
                // Still a deployment step; keep the uuid so `deployed(None)` holds.
                return Ok(r.uuid.clone());
            }
        }
        Ok(r.slug.clone())
    }

    // ---- writes ----

    fn api_post(&self, path: &str, body: Option<&Value>) -> ProviderResult<RawOutput> {
        let mut argv = args(["api", path, "--method", "POST"]);
        if let Some(b) = body {
            argv.push("--input".into());
            argv.push(b.to_string());
        }
        self.exec(argv)
    }

    /// Posts via the raw API (`POST .../pullrequests/ID/comments`, body as `bkt`'s own
    /// `CommentPullRequest` builds it: `content.raw` plus `inline{path,to|from}`), because
    /// `bkt pr comment` does not print the created comment. The body travels as one argv
    /// element (serde_json-escaped), never through a shell.
    fn add_pr_comment(&self, repo: &str, id: u64, c: &NewPrComment) -> ProviderResult<PrComment> {
        let (w, s) = split_repo(repo)?;
        if c.body.trim().is_empty() {
            return Err(ProviderError::Other(
                "refusing to post an empty comment".into(),
            ));
        }
        let mut body = json!({ "content": { "raw": c.body } });
        if let Some(a) = &c.inline {
            if a.path.trim().is_empty() || a.line == 0 {
                return Err(ProviderError::Other(
                    "inline comment needs a file path and a 1-based line".into(),
                ));
            }
            let side = match a.side {
                super::model::DiffSide::New => "to",
                super::model::DiffSide::Old => "from",
            };
            body["inline"] = json!({ "path": a.path, side: a.line });
        }
        self.ensure_cloud()?;
        let o = self.api_post(
            &format!("/repositories/{w}/{s}/pullrequests/{id}/comments"),
            Some(&body),
        )?;
        let posted = Self::parse::<WComment>("created comment", &o.stdout)
            .and_then(|wc| Self::convert("created comment", &o.stdout, wc.into_comment(id)));
        posted.map_err(|e| ProviderError::Other(format!(
            "the comment was probably posted, but bkt's response could not be read (do not retry blindly): {e}"
        )))
    }

    fn approve(&self, repo: &str, id: u64) -> ProviderResult<()> {
        let (w, s) = split_repo(repo)?;
        self.ensure_cloud()?;
        self.exec(args([
            "pr",
            "approve",
            &id.to_string(),
            "--workspace",
            w,
            "--repo",
            s,
        ]))?;
        Ok(())
    }

    /// No `bkt pr request-changes` exists. The comment (if any) is posted first as a general
    /// comment, then `POST .../request-changes` (Bitbucket Cloud REST; UNVERIFIED through
    /// bkt) is sent through the raw API.
    fn request_changes(&self, repo: &str, id: u64, body: &str) -> ProviderResult<()> {
        let (w, s) = split_repo(repo)?;
        self.ensure_cloud()?;
        let mut posted = None;
        if !body.trim().is_empty() {
            let c = NewPrComment {
                body: body.into(),
                inline: None,
            };
            posted = Some(self.add_pr_comment(repo, id, &c)?.id);
        }
        self.api_post(
            &format!("/repositories/{w}/{s}/pullrequests/{id}/request-changes"),
            None,
        )
        .map(|_| ())
        .map_err(|e| match posted {
            Some(cid) => ProviderError::Other(format!(
                "comment {cid} was posted but requesting changes failed: {e}"
            )),
            None => e,
        })
    }

    fn post_pipeline(&self, repo: &str, target: Value) -> ProviderResult<PipelineRun> {
        let (w, s) = split_repo(repo)?;
        let o = self.api_post(
            &format!("/repositories/{w}/{s}/pipelines/"),
            Some(&json!({ "target": target })),
        )?;
        let wp: WPipeline = Self::parse("triggered pipeline", &o.stdout)?;
        Self::convert("triggered pipeline", &o.stdout, wp.into_run(repo))
    }

    /// `POST /repositories/W/S/pipelines/` with the target shape `bkt pipeline run` builds
    /// (`pipeline_ref_target`, `ref_type: branch`, `ref_name`, `selector{custom,pattern}`);
    /// `bkt pipeline run` itself prints only text, so the raw API is used to get the run back.
    /// A pinned `commit` (`commit{type,hash}`) is UNVERIFIED and must be a full 40-char SHA.
    fn trigger_pipeline(&self, repo: &str, spec: &TriggerSpec) -> ProviderResult<PipelineRun> {
        split_repo(repo)?;
        if spec.branch.trim().is_empty() {
            return Err(ProviderError::Other(
                "a branch is required to trigger a pipeline".into(),
            ));
        }
        let mut target = json!({
            "type": "pipeline_ref_target",
            "ref_type": "branch",
            "ref_name": spec.branch,
        });
        if let Some(c) = &spec.commit {
            if c.len() != 40 || !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(ProviderError::Other(format!(
                    "pinning a pipeline to a commit needs the full 40-character SHA, got {c:?}"
                )));
            }
            target["commit"] = json!({ "type": "commit", "hash": c });
        }
        if let Some(name) = &spec.custom_pipeline {
            target["selector"] = json!({ "type": "custom", "pattern": name });
        }
        self.ensure_cloud()?;
        self.post_pipeline(repo, target)
    }

    /// Bitbucket has no rerun endpoint (UNVERIFIED) and neither does `bkt`: this reads the
    /// old run and triggers a new run of the same target (branch or commit, selector).
    /// Pipeline variables are not carried over.
    fn rerun_pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        self.ensure_cloud()?;
        let o = {
            let (w, s) = split_repo(repo)?;
            let enc = encode_pipeline_id(id)?;
            self.api_get(&format!("/repositories/{w}/{s}/pipelines/{enc}"), &[])?
        };
        let wp: WPipeline = Self::parse("pipeline", &o.stdout)?;
        let t = wp.target;
        if !matches!(
            t.kind.as_str(),
            "pipeline_ref_target" | "pipeline_commit_target"
        ) {
            return Err(ProviderError::Unsupported(format!(
                "cannot rerun a pipeline with target type {:?}",
                t.kind
            )));
        }
        let mut target = json!({ "type": t.kind });
        if !t.ref_name.is_empty() {
            let rt = if t.ref_type.is_empty() {
                "branch"
            } else {
                &t.ref_type
            };
            target["ref_type"] = json!(rt);
            target["ref_name"] = json!(t.ref_name);
        }
        if !t.commit.hash.is_empty() {
            target["commit"] = json!({ "type": "commit", "hash": t.commit.hash });
        }
        if !t.selector.kind.is_empty() {
            let mut sel = json!({ "type": t.selector.kind });
            if !t.selector.pattern.is_empty() {
                sel["pattern"] = json!(t.selector.pattern);
            }
            target["selector"] = sel;
        }
        self.post_pipeline(repo, target)
    }
}

/// Read-only Bitbucket Cloud access through `bkt`. Issues no write command.
pub struct BktHost<R: BktRunner = SystemRunner> {
    client: Client<R>,
}

impl BktHost<SystemRunner> {
    /// Does not spawn anything or touch the network.
    pub fn new(_config: &Config) -> ProviderResult<Self> {
        Ok(Self::with_runner(SystemRunner::new()))
    }
}

impl<R: BktRunner> BktHost<R> {
    pub fn with_runner(runner: R) -> Self {
        Self {
            client: Client::new(runner),
        }
    }
}

impl<R: BktRunner> CodeHost for BktHost<R> {
    fn health(&self) -> Health {
        self.client.health()
    }
    fn list_prs(&self, repo: &str, filter: &PrFilter) -> ProviderResult<Vec<Pr>> {
        self.client.list_prs(repo, filter)
    }
    fn pr(&self, repo: &str, id: u64) -> ProviderResult<Pr> {
        self.client.pr(repo, id)
    }
    fn pr_comments(&self, repo: &str, id: u64) -> ProviderResult<Vec<PrComment>> {
        self.client.pr_comments(repo, id)
    }
    fn pipelines(&self, repo: &str, filter: &PipelineFilter) -> ProviderResult<Vec<PipelineRun>> {
        self.client.pipelines(repo, filter)
    }
    fn pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        self.client.pipeline(repo, id)
    }
}

/// Write access. **Only the write gateway may hold this**, after explicit user confirmation.
pub struct BktHostWriter<R: BktRunner = SystemRunner> {
    client: Client<R>,
}

impl BktHostWriter<SystemRunner> {
    /// Does not spawn anything or touch the network. `pub(crate)`: code outside `de-core`
    /// cannot build a writer at all.
    pub(crate) fn new(_config: &Config) -> ProviderResult<Self> {
        Ok(Self::with_runner(SystemRunner::new()))
    }
}

impl<R: BktRunner> BktHostWriter<R> {
    pub(crate) fn with_runner(runner: R) -> Self {
        Self {
            client: Client::new(runner),
        }
    }
}

impl<R: BktRunner> CodeHostWriter for BktHostWriter<R> {
    fn add_pr_comment(
        &self,
        repo: &str,
        id: u64,
        comment: &NewPrComment,
    ) -> ProviderResult<PrComment> {
        self.client.add_pr_comment(repo, id, comment)
    }
    fn approve(&self, repo: &str, id: u64) -> ProviderResult<()> {
        self.client.approve(repo, id)
    }
    fn request_changes(&self, repo: &str, id: u64, body: &str) -> ProviderResult<()> {
        self.client.request_changes(repo, id, body)
    }
    fn trigger_pipeline(&self, repo: &str, spec: &TriggerSpec) -> ProviderResult<PipelineRun> {
        self.client.trigger_pipeline(repo, spec)
    }
    fn rerun_pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        self.client.rerun_pipeline(repo, id)
    }
}

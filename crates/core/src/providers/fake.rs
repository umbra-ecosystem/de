//! In-memory fakes of every provider trait, for tests.
//!
//! Available inside `de-core` tests and, with the `testing` cargo feature, to other crates:
//! `de-core = { workspace = true, features = ["testing"] }` in `[dev-dependencies]`.
//!
//! - [`FakeJira`] implements [`TicketProvider`] and [`TicketWriter`].
//! - [`FakeBitbucket`] implements [`CodeHost`] and [`CodeHostWriter`].
//!
//! Both are cheap `Clone` handles onto shared state, so a test can pass `&fake` to code under
//! test and keep using its own handle to script data and inspect what happened.
//!
//! Every trait call is appended to a [`CallLog`]. Tests assert on it, above all that
//! **no write method was called** (`log.writes().is_empty()`), which is how read-only code
//! such as sync is verified.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use super::error::{ProviderError, ProviderResult};
use super::model::{
    Health, NewPrComment, PipelineFilter, PipelineRun, Pr, PrComment, PrFilter, RemoteComment,
    RemoteTicket, TriggerSpec,
};
use super::traits::{CodeHost, CodeHostWriter, TicketProvider, TicketWriter};
use crate::domain::TicketKey;

/// Method names of the write traits. A call to any of these is a write.
pub const WRITE_METHODS: &[&str] = &[
    "add_comment",
    "transition",
    "add_pr_comment",
    "approve",
    "request_changes",
    "trigger_pipeline",
    "rerun_pipeline",
];

/// One recorded trait call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// The trait method, e.g. `search` or `approve`.
    pub method: &'static str,
    /// The arguments, rendered for assertions, e.g. `acme/web #7`.
    pub args: String,
}

impl Call {
    pub fn is_write(&self) -> bool {
        WRITE_METHODS.contains(&self.method)
    }
}

/// A shared, ordered record of the calls a fake received.
#[derive(Debug, Clone, Default)]
pub struct CallLog(Arc<Mutex<Vec<Call>>>);

impl CallLog {
    fn record(&self, method: &'static str, args: impl Into<String>) {
        lock(&self.0).push(Call {
            method,
            args: args.into(),
        });
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<Call> {
        lock(&self.0).clone()
    }

    /// The calls to write methods. Empty means the code under test only read.
    pub fn writes(&self) -> Vec<Call> {
        self.calls().into_iter().filter(Call::is_write).collect()
    }

    /// How many times `method` was called.
    pub fn count(&self, method: &str) -> usize {
        self.calls().iter().filter(|c| c.method == method).count()
    }

    /// Arguments of every call to `method`.
    pub fn args_of(&self, method: &str) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|c| c.method == method)
            .map(|c| c.args)
            .collect()
    }

    pub fn clear(&self) {
        lock(&self.0).clear();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ------------------------------------------------------------------- Jira

#[derive(Default)]
struct JiraState {
    tickets: BTreeMap<TicketKey, RemoteTicket>,
    searches: HashMap<String, Vec<TicketKey>>,
    comments: BTreeMap<TicketKey, Vec<RemoteComment>>,
    health: Option<Health>,
    /// Fails every call except `health`.
    failure: Option<ProviderError>,
    fail_search: Option<ProviderError>,
    fail_keys: HashMap<TicketKey, ProviderError>,
    next_id: u64,
}

/// In-memory Jira: scriptable tickets, searches, comments and failures.
#[derive(Clone, Default)]
pub struct FakeJira {
    state: Arc<Mutex<JiraState>>,
    log: CallLog,
}

impl FakeJira {
    pub fn new() -> Self {
        Self::default()
    }

    /// The call log shared by every clone.
    pub fn log(&self) -> CallLog {
        self.log.clone()
    }

    /// Makes `ticket` known to `get`.
    pub fn add_ticket(&self, ticket: RemoteTicket) -> &Self {
        lock(&self.state).tickets.insert(ticket.key.clone(), ticket);
        self
    }

    /// Scripts `search(jql)` to return exactly `tickets` (which also become known to `get`).
    pub fn on_search(&self, jql: &str, tickets: Vec<RemoteTicket>) -> &Self {
        let mut state = lock(&self.state);
        let keys = tickets.iter().map(|t| t.key.clone()).collect();
        for t in tickets {
            state.tickets.insert(t.key.clone(), t);
        }
        state.searches.insert(jql.into(), keys);
        self
    }

    /// Scripts the comments of a ticket.
    pub fn set_comments(&self, key: &TicketKey, comments: Vec<RemoteComment>) -> &Self {
        lock(&self.state).comments.insert(key.clone(), comments);
        self
    }

    /// Removes a ticket, so `get` says not found.
    pub fn remove_ticket(&self, key: &TicketKey) -> &Self {
        lock(&self.state).tickets.remove(key);
        self
    }

    /// Every call except `health` fails with a network error.
    pub fn set_offline(&self) -> &Self {
        self.set_failure(Some(ProviderError::Network("offline".into())))
    }

    /// Every call except `health` fails as not logged in; `health` reports it too.
    pub fn set_unauthenticated(&self) -> &Self {
        let mut h = Health::ok("fake");
        h.authenticated = false;
        h.detail = "not logged in".into();
        lock(&self.state).health = Some(h);
        self.set_failure(Some(ProviderError::not_authenticated(
            "acli",
            "not logged in",
        )))
    }

    /// Fails every call except `health` with `error`; `None` restores normal behaviour.
    pub fn set_failure(&self, error: Option<ProviderError>) -> &Self {
        lock(&self.state).failure = error;
        self
    }

    /// Fails only `search`.
    pub fn fail_search(&self, error: ProviderError) -> &Self {
        lock(&self.state).fail_search = Some(error);
        self
    }

    /// Fails `get` and `comments` (and writes) for one ticket.
    pub fn fail_key(&self, key: &TicketKey, error: ProviderError) -> &Self {
        lock(&self.state).fail_keys.insert(key.clone(), error);
        self
    }

    pub fn set_health(&self, health: Health) -> &Self {
        lock(&self.state).health = Some(health);
        self
    }

    /// The tickets as the fake currently holds them (writes such as `transition` show here).
    pub fn ticket(&self, key: &TicketKey) -> Option<RemoteTicket> {
        lock(&self.state).tickets.get(key).cloned()
    }

    fn check(&self, key: Option<&TicketKey>) -> ProviderResult<()> {
        let state = lock(&self.state);
        if let Some(e) = &state.failure {
            return Err(e.clone());
        }
        if let Some(e) = key.and_then(|k| state.fail_keys.get(k)) {
            return Err(e.clone());
        }
        Ok(())
    }
}

impl TicketProvider for FakeJira {
    fn health(&self) -> Health {
        self.log.record("health", "");
        lock(&self.state)
            .health
            .clone()
            .unwrap_or_else(|| Health::ok("fake 1.0"))
    }

    fn search(&self, jql: &str) -> ProviderResult<Vec<RemoteTicket>> {
        self.log.record("search", jql);
        self.check(None)?;
        let state = lock(&self.state);
        if let Some(e) = &state.fail_search {
            return Err(e.clone());
        }
        Ok(state
            .searches
            .get(jql)
            .map(|keys| {
                keys.iter()
                    .filter_map(|k| state.tickets.get(k).cloned())
                    .collect()
            })
            .unwrap_or_default())
    }

    fn get(&self, key: &TicketKey) -> ProviderResult<RemoteTicket> {
        self.log.record("get", key.as_str());
        self.check(Some(key))?;
        lock(&self.state)
            .tickets
            .get(key)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(key.to_string()))
    }

    fn comments(&self, key: &TicketKey) -> ProviderResult<Vec<RemoteComment>> {
        self.log.record("comments", key.as_str());
        self.check(Some(key))?;
        Ok(lock(&self.state)
            .comments
            .get(key)
            .cloned()
            .unwrap_or_default())
    }
}

impl TicketWriter for FakeJira {
    fn add_comment(&self, key: &TicketKey, body: &str) -> ProviderResult<RemoteComment> {
        self.log.record("add_comment", format!("{key}: {body}"));
        self.check(Some(key))?;
        let mut state = lock(&self.state);
        state.next_id += 1;
        let comment = RemoteComment {
            id: format!("written-{}", state.next_id),
            ticket: key.clone(),
            author_account_id: "fake-self".into(),
            author_name: "Fake".into(),
            body_text: body.into(),
            mentions: Vec::new(),
            created_at: 0,
        };
        state
            .comments
            .entry(key.clone())
            .or_default()
            .push(comment.clone());
        Ok(comment)
    }

    fn transition(&self, key: &TicketKey, to_status: &str) -> ProviderResult<()> {
        self.log
            .record("transition", format!("{key} -> {to_status}"));
        self.check(Some(key))?;
        match lock(&self.state).tickets.get_mut(key) {
            Some(t) => {
                t.status = to_status.into();
                Ok(())
            }
            None => Err(ProviderError::NotFound(key.to_string())),
        }
    }
}

// -------------------------------------------------------------- Bitbucket

#[derive(Default)]
struct HostState {
    prs: Vec<Pr>,
    pr_comments: BTreeMap<(String, u64), Vec<PrComment>>,
    pipelines: Vec<PipelineRun>,
    strip_steps_in_lists: bool,
    health: Option<Health>,
    failure: Option<ProviderError>,
    fail_repos: HashMap<String, ProviderError>,
    next_id: u64,
}

/// In-memory Bitbucket: scriptable PRs, comments, pipelines and failures (global or per repo).
#[derive(Clone, Default)]
pub struct FakeBitbucket {
    state: Arc<Mutex<HostState>>,
    log: CallLog,
}

impl FakeBitbucket {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn log(&self) -> CallLog {
        self.log.clone()
    }

    /// Adds a PR, replacing one with the same `(repo, id)`.
    pub fn add_pr(&self, pr: Pr) -> &Self {
        let mut state = lock(&self.state);
        state.prs.retain(|p| !(p.repo == pr.repo && p.id == pr.id));
        state.prs.push(pr);
        self
    }

    /// Removes a PR, as if it vanished from the host.
    pub fn remove_pr(&self, repo: &str, id: u64) -> &Self {
        lock(&self.state)
            .prs
            .retain(|p| !(p.repo == repo && p.id == id));
        self
    }

    pub fn set_pr_comments(&self, repo: &str, id: u64, comments: Vec<PrComment>) -> &Self {
        lock(&self.state)
            .pr_comments
            .insert((repo.into(), id), comments);
        self
    }

    /// Adds a pipeline run, replacing one with the same `(repo, id)`.
    pub fn add_pipeline(&self, run: PipelineRun) -> &Self {
        let mut state = lock(&self.state);
        state
            .pipelines
            .retain(|p| !(p.repo == run.repo && p.id == run.id));
        state.pipelines.push(run);
        self
    }

    /// Makes `pipelines` return runs without steps, as real list endpoints may, so callers
    /// must call `pipeline` for the steps.
    pub fn strip_steps_in_lists(&self, strip: bool) -> &Self {
        lock(&self.state).strip_steps_in_lists = strip;
        self
    }

    pub fn set_offline(&self) -> &Self {
        self.set_failure(Some(ProviderError::Network("offline".into())))
    }

    pub fn set_unauthenticated(&self) -> &Self {
        let mut h = Health::ok("fake");
        h.authenticated = false;
        h.detail = "not logged in".into();
        lock(&self.state).health = Some(h);
        self.set_failure(Some(ProviderError::not_authenticated(
            "bkt",
            "not logged in",
        )))
    }

    /// Fails every call except `health`; `None` restores normal behaviour.
    pub fn set_failure(&self, error: Option<ProviderError>) -> &Self {
        lock(&self.state).failure = error;
        self
    }

    /// Fails every call about one repo (`workspace/slug`).
    pub fn fail_repo(&self, repo: &str, error: ProviderError) -> &Self {
        lock(&self.state).fail_repos.insert(repo.into(), error);
        self
    }

    pub fn clear_repo_failures(&self) -> &Self {
        lock(&self.state).fail_repos.clear();
        self
    }

    pub fn set_health(&self, health: Health) -> &Self {
        lock(&self.state).health = Some(health);
        self
    }

    /// The PR as the fake currently holds it (writes such as `approve` show here).
    pub fn pr_now(&self, repo: &str, id: u64) -> Option<Pr> {
        lock(&self.state)
            .prs
            .iter()
            .find(|p| p.repo == repo && p.id == id)
            .cloned()
    }

    fn check(&self, repo: &str) -> ProviderResult<()> {
        let state = lock(&self.state);
        if let Some(e) = &state.failure {
            return Err(e.clone());
        }
        if let Some(e) = state.fail_repos.get(repo) {
            return Err(e.clone());
        }
        Ok(())
    }
}

impl CodeHost for FakeBitbucket {
    fn health(&self) -> Health {
        self.log.record("health", "");
        lock(&self.state)
            .health
            .clone()
            .unwrap_or_else(|| Health::ok("fake 1.0"))
    }

    fn list_prs(&self, repo: &str, filter: &PrFilter) -> ProviderResult<Vec<Pr>> {
        self.log.record("list_prs", format!("{repo} {filter:?}"));
        self.check(repo)?;
        let text = filter.text.as_deref().map(str::to_ascii_lowercase);
        Ok(lock(&self.state)
            .prs
            .iter()
            .filter(|p| p.repo == repo)
            .filter(|p| filter.state.is_none_or(|s| p.state == s))
            .filter(|p| {
                text.as_deref().is_none_or(|t| {
                    p.title.to_ascii_lowercase().contains(t)
                        || p.source_branch.to_ascii_lowercase().contains(t)
                })
            })
            .cloned()
            .collect())
    }

    fn pr(&self, repo: &str, id: u64) -> ProviderResult<Pr> {
        self.log.record("pr", format!("{repo} #{id}"));
        self.check(repo)?;
        lock(&self.state)
            .prs
            .iter()
            .find(|p| p.repo == repo && p.id == id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("{repo} #{id}")))
    }

    fn pr_comments(&self, repo: &str, id: u64) -> ProviderResult<Vec<PrComment>> {
        self.log.record("pr_comments", format!("{repo} #{id}"));
        self.check(repo)?;
        Ok(lock(&self.state)
            .pr_comments
            .get(&(repo.to_string(), id))
            .cloned()
            .unwrap_or_default())
    }

    fn pipelines(&self, repo: &str, filter: &PipelineFilter) -> ProviderResult<Vec<PipelineRun>> {
        self.log.record("pipelines", format!("{repo} {filter:?}"));
        self.check(repo)?;
        let state = lock(&self.state);
        let mut runs: Vec<PipelineRun> = state
            .pipelines
            .iter()
            .filter(|r| r.repo == repo)
            .filter(|r| filter.branch.as_deref().is_none_or(|b| r.branch == b))
            .filter(|r| filter.commit.as_deref().is_none_or(|c| r.is_for_commit(c)))
            .cloned()
            .collect();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(limit) = filter.limit {
            runs.truncate(limit);
        }
        if state.strip_steps_in_lists {
            for r in &mut runs {
                r.steps.clear();
            }
        }
        Ok(runs)
    }

    fn pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        self.log.record("pipeline", format!("{repo} {id}"));
        self.check(repo)?;
        lock(&self.state)
            .pipelines
            .iter()
            .find(|r| r.repo == repo && r.id == id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("{repo} pipeline {id}")))
    }
}

impl CodeHostWriter for FakeBitbucket {
    fn add_pr_comment(
        &self,
        repo: &str,
        id: u64,
        comment: &NewPrComment,
    ) -> ProviderResult<PrComment> {
        self.log
            .record("add_pr_comment", format!("{repo} #{id}: {}", comment.body));
        self.check(repo)?;
        let mut state = lock(&self.state);
        state.next_id += 1;
        let written = PrComment {
            id: 1_000_000 + state.next_id,
            pr: id,
            author: "fake-self".into(),
            body: comment.body.clone(),
            inline: comment.inline.clone(),
            created_at: 0,
        };
        state
            .pr_comments
            .entry((repo.into(), id))
            .or_default()
            .push(written.clone());
        Ok(written)
    }

    fn approve(&self, repo: &str, id: u64) -> ProviderResult<()> {
        self.log.record("approve", format!("{repo} #{id}"));
        self.check(repo)
    }

    fn request_changes(&self, repo: &str, id: u64, body: &str) -> ProviderResult<()> {
        self.log
            .record("request_changes", format!("{repo} #{id}: {body}"));
        self.check(repo)
    }

    fn trigger_pipeline(&self, repo: &str, spec: &TriggerSpec) -> ProviderResult<PipelineRun> {
        self.log
            .record("trigger_pipeline", format!("{repo} {}", spec.branch));
        self.check(repo)?;
        let mut state = lock(&self.state);
        state.next_id += 1;
        let run = PipelineRun {
            repo: repo.into(),
            id: format!("triggered-{}", state.next_id),
            number: None,
            state: super::model::PipelineState::Pending,
            branch: spec.branch.clone(),
            commit: spec.commit.clone().unwrap_or_default(),
            created_at: 0,
            completed_at: None,
            url: String::new(),
            steps: Vec::new(),
        };
        state.pipelines.push(run.clone());
        Ok(run)
    }

    fn rerun_pipeline(&self, repo: &str, id: &str) -> ProviderResult<PipelineRun> {
        self.log.record("rerun_pipeline", format!("{repo} {id}"));
        self.check(repo)?;
        let mut state = lock(&self.state);
        let original = state
            .pipelines
            .iter()
            .find(|r| r.repo == repo && r.id == id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("{repo} pipeline {id}")))?;
        state.next_id += 1;
        let run = PipelineRun {
            id: format!("rerun-{}", state.next_id),
            state: super::model::PipelineState::Pending,
            completed_at: None,
            steps: Vec::new(),
            ..original
        };
        state.pipelines.push(run.clone());
        Ok(run)
    }
}

/// Builders for test data, to keep tests short.
pub mod build {
    use super::super::model::{
        PipelineRun, PipelineState, PipelineStep, Pr, PrState, RemoteComment, RemoteTicket,
    };
    use crate::domain::TicketKey;

    pub fn key(s: &str) -> TicketKey {
        s.parse().expect("valid ticket key")
    }

    pub fn ticket(k: &str, status: &str) -> RemoteTicket {
        RemoteTicket {
            key: key(k),
            title: format!("Title of {k}"),
            status: status.into(),
            priority: Some("Medium".into()),
            assignee: None,
            url: Some(format!("https://jira.test/browse/{k}")),
            updated_at: 100,
        }
    }

    pub fn comment(k: &str, id: &str, mentions: &[&str]) -> RemoteComment {
        RemoteComment {
            id: id.into(),
            ticket: key(k),
            author_account_id: "someone".into(),
            author_name: "Someone".into(),
            body_text: format!("comment {id}"),
            mentions: mentions.iter().map(|m| String::from(*m)).collect(),
            created_at: 100,
        }
    }

    pub fn pr(repo: &str, id: u64, source: &str, destination: &str) -> Pr {
        Pr {
            repo: repo.into(),
            id,
            title: format!("PR {id}"),
            state: PrState::Open,
            source_branch: source.into(),
            destination_branch: destination.into(),
            author: "author".into(),
            reviewers: Vec::new(),
            url: format!("https://bb.test/{repo}/pull-requests/{id}"),
            updated_at: 100,
        }
    }

    pub fn run(repo: &str, id: &str, branch: &str, commit: &str, at: i64) -> PipelineRun {
        PipelineRun {
            repo: repo.into(),
            id: id.into(),
            number: None,
            state: PipelineState::Succeeded,
            branch: branch.into(),
            commit: commit.into(),
            created_at: at,
            completed_at: Some(at + 10),
            url: format!("https://bb.test/{repo}/pipelines/{id}"),
            steps: vec![PipelineStep {
                name: "deploy".into(),
                state: PipelineState::Succeeded,
                deployment_environment: Some("alpha".into()),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;
    use crate::providers::{PrState, ProviderErrorKind};

    #[test]
    fn jira_fake_serves_scripted_data_and_logs_calls() {
        let jira = FakeJira::new();
        jira.on_search("q", vec![ticket("PROJ-1", "In Review")]);
        jira.set_comments(&key("PROJ-1"), vec![comment("PROJ-1", "c1", &["me"])]);

        assert_eq!(jira.search("q").unwrap().len(), 1);
        assert!(jira.search("other").unwrap().is_empty());
        assert_eq!(jira.get(&key("PROJ-1")).unwrap().status, "In Review");
        assert_eq!(
            jira.get(&key("PROJ-2")).unwrap_err().kind(),
            ProviderErrorKind::NotFound
        );
        assert_eq!(jira.comments(&key("PROJ-1")).unwrap()[0].mentions, ["me"]);

        let log = jira.log();
        assert_eq!(log.count("search"), 2);
        assert_eq!(log.args_of("get"), ["PROJ-1", "PROJ-2"]);
        assert!(log.writes().is_empty());
    }

    #[test]
    fn jira_writes_are_logged_as_writes_and_change_state() {
        let jira = FakeJira::new();
        jira.add_ticket(ticket("PROJ-1", "In Review"));
        jira.add_comment(&key("PROJ-1"), "deployed").unwrap();
        jira.transition(&key("PROJ-1"), "Alpha Testing").unwrap();

        assert_eq!(jira.log().writes().len(), 2);
        assert_eq!(jira.ticket(&key("PROJ-1")).unwrap().status, "Alpha Testing");
        assert_eq!(jira.comments(&key("PROJ-1")).unwrap().len(), 1);
    }

    #[test]
    fn jira_failures_are_scriptable() {
        let jira = FakeJira::new();
        jira.add_ticket(ticket("PROJ-1", "x"));
        jira.set_offline();
        assert_eq!(
            jira.search("q").unwrap_err().kind(),
            ProviderErrorKind::Network
        );
        jira.set_failure(None);
        jira.set_unauthenticated();
        assert!(!jira.health().authenticated);
        assert_eq!(
            jira.get(&key("PROJ-1")).unwrap_err().kind(),
            ProviderErrorKind::NotAuthenticated
        );
        jira.set_failure(None);
        jira.fail_key(&key("PROJ-1"), ProviderError::Other("boom".into()));
        assert!(jira.get(&key("PROJ-1")).is_err());
        assert!(jira.search("q").is_ok());
    }

    #[test]
    fn bitbucket_fake_filters_prs_and_pipelines() {
        let bb = FakeBitbucket::new();
        bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
        let mut merged = pr("acme/web", 2, "feature/PROJ-2", "develop");
        merged.state = PrState::Merged;
        bb.add_pr(merged);
        bb.add_pr(pr("acme/api", 3, "feature/PROJ-1", "develop"));

        assert_eq!(bb.list_prs("acme/web", &PrFilter::open()).unwrap().len(), 1);
        assert_eq!(
            bb.list_prs("acme/web", &PrFilter::default()).unwrap().len(),
            2
        );
        let text = PrFilter {
            state: None,
            text: Some("proj-2".into()),
        };
        assert_eq!(bb.list_prs("acme/web", &text).unwrap()[0].id, 2);
        assert_eq!(
            bb.pr("acme/web", 9).unwrap_err().kind(),
            ProviderErrorKind::NotFound
        );

        bb.add_pipeline(run("acme/web", "a", "uat", "abcdef1234", 1));
        bb.add_pipeline(run("acme/web", "b", "uat", "1234567890", 2));
        let by_commit = PipelineFilter {
            commit: Some("abcdef1".into()),
            ..Default::default()
        };
        assert_eq!(bb.pipelines("acme/web", &by_commit).unwrap()[0].id, "a");
        let limited = PipelineFilter {
            limit: Some(1),
            ..Default::default()
        };
        assert_eq!(bb.pipelines("acme/web", &limited).unwrap()[0].id, "b");
        bb.strip_steps_in_lists(true);
        assert!(
            bb.pipelines("acme/web", &limited).unwrap()[0]
                .steps
                .is_empty()
        );
        assert!(!bb.pipeline("acme/web", "b").unwrap().steps.is_empty());
    }

    #[test]
    fn bitbucket_failures_can_target_one_repo_and_writes_are_logged() {
        let bb = FakeBitbucket::new();
        bb.add_pr(pr("acme/web", 1, "b", "develop"));
        bb.fail_repo("acme/api", ProviderError::Other("boom".into()));
        assert!(bb.list_prs("acme/api", &PrFilter::open()).is_err());
        assert!(bb.list_prs("acme/web", &PrFilter::open()).is_ok());
        bb.set_offline();
        assert!(bb.list_prs("acme/web", &PrFilter::open()).is_err());
        bb.set_failure(None);

        assert!(bb.log().writes().is_empty());
        bb.approve("acme/web", 1).unwrap();
        bb.request_changes("acme/web", 1, "no").unwrap();
        bb.add_pr_comment(
            "acme/web",
            1,
            &NewPrComment {
                body: "hi".into(),
                inline: None,
            },
        )
        .unwrap();
        let run = bb
            .trigger_pipeline(
                "acme/web",
                &TriggerSpec {
                    branch: "uat".into(),
                    commit: None,
                    custom_pipeline: None,
                },
            )
            .unwrap();
        bb.rerun_pipeline("acme/web", &run.id).unwrap();
        let methods: Vec<_> = bb.log().writes().iter().map(|c| c.method).collect();
        assert_eq!(
            methods,
            [
                "approve",
                "request_changes",
                "add_pr_comment",
                "trigger_pipeline",
                "rerun_pipeline"
            ]
        );
    }
}

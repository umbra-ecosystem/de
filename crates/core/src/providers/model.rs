//! Data returned by providers. Plain, serde-serializable values with no behaviour beyond
//! small pure helpers.
//!
//! Conventions shared by every type here:
//!
//! - Times are unix seconds (`i64`), UTC. Adapters convert whatever the tool prints.
//! - A `repo` is always the code-host path `workspace/slug` (for example `acme/web`), never
//!   the local project name; [`crate::sync::HostedRepo`] maps between the two.
//! - "Account" values (`author`, reviewers, mentions) are stable account ids when the tool
//!   exposes them, otherwise display names; adapters must be consistent within one system.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::TicketKey;

/// A Jira ticket as reported by the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTicket {
    pub key: TicketKey,
    pub title: String,
    /// The Jira workflow status name, exactly as Jira spells it (`In Review`).
    pub status: String,
    pub priority: Option<String>,
    /// Display name of the assignee.
    pub assignee: Option<String>,
    /// Browser URL of the ticket.
    pub url: Option<String>,
    pub updated_at: i64,
}

/// A Jira comment, flattened to text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteComment {
    /// Jira's comment id; unique within a ticket.
    pub id: String,
    pub ticket: TicketKey,
    pub author_account_id: String,
    pub author_name: String,
    /// The comment rendered as plain text (mentions appear as `@Name`).
    pub body_text: String,
    /// Account ids mentioned in the comment. This is what "@mentioned" is decided on.
    pub mentions: Vec<String>,
    pub created_at: i64,
}

/// Lifecycle state of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Declined,
    Superseded,
}

impl PrState {
    pub const ALL: &'static [PrState] = &[
        PrState::Open,
        PrState::Merged,
        PrState::Declined,
        PrState::Superseded,
    ];

    /// The text stored in `cache.db`.
    pub fn as_str(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Merged => "merged",
            PrState::Declined => "declined",
            PrState::Superseded => "superseded",
        }
    }

    pub fn parse(text: &str) -> Option<PrState> {
        Self::ALL.iter().copied().find(|s| s.as_str() == text)
    }
}

impl fmt::Display for PrState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One reviewer of a PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    pub account: String,
    pub approved: bool,
    pub changes_requested: bool,
}

/// A pull request. `(repo, id)` identifies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pr {
    /// `workspace/slug`.
    pub repo: String,
    pub id: u64,
    pub title: String,
    pub state: PrState,
    pub source_branch: String,
    /// Where the PR merges into. Review diffs and the hotfix rule read this, never a guess.
    pub destination_branch: String,
    pub author: String,
    pub reviewers: Vec<Reviewer>,
    pub url: String,
    pub updated_at: i64,
}

/// Which side of a diff an inline comment is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffSide {
    /// The destination (removed or unchanged old) side.
    Old,
    /// The source (added or new) side.
    New,
}

impl DiffSide {
    pub fn as_str(self) -> &'static str {
        match self {
            DiffSide::Old => "old",
            DiffSide::New => "new",
        }
    }

    pub fn parse(text: &str) -> Option<DiffSide> {
        match text {
            "old" => Some(DiffSide::Old),
            "new" => Some(DiffSide::New),
            _ => None,
        }
    }
}

/// Where an inline comment sits in a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineAnchor {
    pub path: String,
    /// 1-based line number on `side`.
    pub line: u32,
    pub side: DiffSide,
}

/// A comment on a PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrComment {
    /// The host's comment id; unique within a PR.
    pub id: u64,
    pub pr: u64,
    pub author: String,
    pub body: String,
    /// `None` for a general comment.
    pub inline: Option<InlineAnchor>,
    pub created_at: i64,
}

/// State of a pipeline run or of one of its steps.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Stopped,
    /// Anything the adapter cannot map (`PAUSED`, `EXPIRED`, ...), kept verbatim.
    Other(String),
}

impl PipelineState {
    /// The text stored in `cache.db`; [`PipelineState::from_db`] is its inverse.
    pub fn to_db(&self) -> String {
        match self {
            PipelineState::Pending => "pending".into(),
            PipelineState::Running => "running".into(),
            PipelineState::Succeeded => "succeeded".into(),
            PipelineState::Failed => "failed".into(),
            PipelineState::Stopped => "stopped".into(),
            PipelineState::Other(s) => format!("other:{s}"),
        }
    }

    pub fn from_db(text: &str) -> PipelineState {
        match text {
            "pending" => PipelineState::Pending,
            "running" => PipelineState::Running,
            "succeeded" => PipelineState::Succeeded,
            "failed" => PipelineState::Failed,
            "stopped" => PipelineState::Stopped,
            other => PipelineState::Other(other.strip_prefix("other:").unwrap_or(other).into()),
        }
    }

    /// Whether the run or step can still change.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            PipelineState::Succeeded | PipelineState::Failed | PipelineState::Stopped
        )
    }
}

impl fmt::Display for PipelineState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineState::Other(s) => write!(f, "{s}"),
            other => f.write_str(&other.to_db()),
        }
    }
}

/// One step of a pipeline run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineStep {
    pub name: String,
    pub state: PipelineState,
    /// Set on deployment steps: the environment deployed to (`alpha`, `production`, ...).
    pub deployment_environment: Option<String>,
}

/// A pipeline run. `(repo, id)` identifies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineRun {
    /// `workspace/slug`.
    pub repo: String,
    /// The host's id (a UUID on Bitbucket Cloud); opaque to `de`.
    pub id: String,
    /// The human build number, when the host has one.
    pub number: Option<u64>,
    pub state: PipelineState,
    pub branch: String,
    /// Full commit SHA the run built.
    pub commit: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub url: String,
    /// May be empty in list results; [`CodeHost::pipeline`](super::CodeHost::pipeline) fills it.
    pub steps: Vec<PipelineStep>,
}

impl PipelineRun {
    /// Whether a deployment step succeeded.
    ///
    /// With `environment` set, only steps deploying to that environment count (compared
    /// case-insensitively); with `None`, any step that has a deployment environment counts.
    pub fn deployed(&self, environment: Option<&str>) -> bool {
        self.steps.iter().any(|s| {
            s.state == PipelineState::Succeeded
                && match (&s.deployment_environment, environment) {
                    (Some(_), None) => true,
                    (Some(have), Some(want)) => have.eq_ignore_ascii_case(want),
                    (None, _) => false,
                }
        })
    }

    /// Whether this run built `commit` (either side may be an abbreviated SHA of at least
    /// 7 characters).
    pub fn is_for_commit(&self, commit: &str) -> bool {
        commit_matches(&self.commit, commit)
    }
}

/// Whether two SHAs name the same commit, allowing abbreviations of at least 7 characters.
pub fn commit_matches(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    if a.len() < 7 || b.len() < 7 {
        return a == b;
    }
    a.starts_with(&b) || b.starts_with(&a)
}

/// What a provider reports about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub installed: bool,
    pub version: Option<String>,
    pub authenticated: bool,
    /// One human-readable line: what is wrong, or what was checked.
    pub detail: String,
    /// Whether `version` is at least the minimum `de` was tested against.
    pub meets_minimum: bool,
}

impl Health {
    /// Everything fine.
    pub fn ok(version: impl Into<String>) -> Self {
        Self {
            installed: true,
            version: Some(version.into()),
            authenticated: true,
            detail: "ready".into(),
            meets_minimum: true,
        }
    }

    /// The tool is missing.
    pub fn not_installed(detail: impl Into<String>) -> Self {
        Self {
            installed: false,
            version: None,
            authenticated: false,
            detail: detail.into(),
            meets_minimum: false,
        }
    }

    /// Usable: installed, logged in and new enough.
    pub fn is_ready(&self) -> bool {
        self.installed && self.authenticated && self.meets_minimum
    }
}

/// Filter for [`CodeHost::list_prs`](super::CodeHost::list_prs). `None` fields do not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrFilter {
    pub state: Option<PrState>,
    /// Free text matched against title and branch by the host.
    pub text: Option<String>,
}

impl PrFilter {
    pub fn open() -> Self {
        Self {
            state: Some(PrState::Open),
            text: None,
        }
    }
}

/// Filter for [`CodeHost::pipelines`](super::CodeHost::pipelines), newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineFilter {
    pub branch: Option<String>,
    /// Full or abbreviated SHA.
    pub commit: Option<String>,
    pub limit: Option<usize>,
}

/// A comment to post on a PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPrComment {
    pub body: String,
    /// `None` posts a general comment.
    pub inline: Option<InlineAnchor>,
}

/// What to run when triggering a pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSpec {
    pub branch: String,
    /// Pin the run to a commit on `branch`; `None` runs the branch head.
    pub commit: Option<String>,
    /// Name of a custom pipeline; `None` runs the branch's default pipeline.
    pub custom_pipeline: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: Vec<PipelineStep>) -> PipelineRun {
        PipelineRun {
            repo: "acme/web".into(),
            id: "{1}".into(),
            number: Some(1),
            state: PipelineState::Succeeded,
            branch: "uat".into(),
            commit: "abcdef1234567890".into(),
            created_at: 1,
            completed_at: Some(2),
            url: String::new(),
            steps,
        }
    }

    fn step(name: &str, state: PipelineState, env: Option<&str>) -> PipelineStep {
        PipelineStep {
            name: name.into(),
            state,
            deployment_environment: env.map(String::from),
        }
    }

    #[test]
    fn deployed_needs_a_succeeded_deployment_step() {
        let r = run(vec![
            step("build", PipelineState::Succeeded, None),
            step("deploy alpha", PipelineState::Succeeded, Some("Alpha")),
        ]);
        assert!(r.deployed(None));
        assert!(r.deployed(Some("alpha")));
        assert!(!r.deployed(Some("production")));

        let failed = run(vec![step("deploy", PipelineState::Failed, Some("alpha"))]);
        assert!(!failed.deployed(None));
        // A succeeded step without an environment is a build, not a deployment.
        assert!(!run(vec![step("build", PipelineState::Succeeded, None)]).deployed(None));
        assert!(!run(vec![]).deployed(None));
    }

    #[test]
    fn commits_match_by_prefix_but_not_when_too_short() {
        let r = run(vec![]);
        assert!(r.is_for_commit("abcdef1234567890"));
        assert!(r.is_for_commit("ABCDEF1"));
        assert!(!r.is_for_commit("abcdef2"));
        assert!(!r.is_for_commit("abc"));
    }

    #[test]
    fn pipeline_state_text_round_trips() {
        for s in [
            PipelineState::Pending,
            PipelineState::Running,
            PipelineState::Succeeded,
            PipelineState::Failed,
            PipelineState::Stopped,
            PipelineState::Other("PAUSED".into()),
        ] {
            assert_eq!(PipelineState::from_db(&s.to_db()), s);
        }
        assert!(PipelineState::Failed.is_finished());
        assert!(!PipelineState::Running.is_finished());
    }

    #[test]
    fn pr_state_text_round_trips_and_serde_matches() {
        for s in PrState::ALL {
            assert_eq!(PrState::parse(s.as_str()), Some(*s));
            assert_eq!(
                serde_json::to_string(s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
        assert_eq!(PrState::parse("closed"), None);
    }

    #[test]
    fn models_serialize_to_json_and_back() {
        let pr = Pr {
            repo: "acme/web".into(),
            id: 7,
            title: "PROJ-1 thing".into(),
            state: PrState::Open,
            source_branch: "feature/PROJ-1".into(),
            destination_branch: "develop".into(),
            author: "a".into(),
            reviewers: vec![Reviewer {
                account: "r".into(),
                approved: true,
                changes_requested: false,
            }],
            url: "https://x".into(),
            updated_at: 5,
        };
        let back: Pr = serde_json::from_str(&serde_json::to_string(&pr).unwrap()).unwrap();
        assert_eq!(back, pr);
    }
}

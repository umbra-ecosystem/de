//! Everything the rules may look at, as plain data.
//!
//! A [`Snapshot`] is built by [`Snapshot::load`](super::load) (the impure part: SQLite, git)
//! or by hand in tests. Nothing here does I/O, and nothing that needs git (dirty trees,
//! `needs_remerge`, branch tips) is computed by a rule: the loader gathers it into these
//! fields.

use std::collections::BTreeMap;

use crate::{
    config::JiraStatuses,
    domain::{LocalStatus, TicketKey, TicketKind},
    integration::DeployState,
    providers::{Pr, PrState},
};

/// The Jira status names of this workflow (defaults filled in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusNames {
    pub review: String,
    pub alpha_testing: String,
    pub uat: String,
    pub returned: String,
    pub done: Vec<String>,
}

impl Default for StatusNames {
    fn default() -> Self {
        Self::from(&JiraStatuses::default())
    }
}

impl From<&JiraStatuses> for StatusNames {
    fn from(s: &JiraStatuses) -> Self {
        Self {
            review: s.review_name().into(),
            alpha_testing: s.alpha_testing_name().into(),
            uat: s.uat_name().into(),
            returned: s.returned_name().into(),
            done: s.done_names().into_iter().map(String::from).collect(),
        }
    }
}

/// Jira status names compare case-insensitively and ignore surrounding space.
pub fn status_is(actual: Option<&str>, wanted: &str) -> bool {
    actual.is_some_and(|a| a.trim().eq_ignore_ascii_case(wanted.trim()))
}

/// Whether a writer the gateway needs is usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready,
    /// Not installed, not logged in, not built: the reason, as the provider reports it.
    Unavailable(String),
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Ready)
    }
}

/// Which external writers can be used right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writers {
    /// Comments and transitions in Jira.
    pub jira: Availability,
    /// PR approvals and pipeline re-runs on the code host.
    pub code_host: Availability,
}

impl Default for Writers {
    fn default() -> Self {
        Self {
            jira: Availability::Ready,
            code_host: Availability::Ready,
        }
    }
}

/// When a sync source last ran (a row of `sync_state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceState {
    pub source: String,
    pub last_ok_at: Option<i64>,
    pub last_attempt_at: i64,
    pub last_error: Option<String>,
}

/// The sync sources that should exist, and what is known about each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSnapshot {
    /// `jira` when configured, and `bitbucket:<repo>` per hosted repo.
    pub expected: Vec<String>,
    pub states: Vec<SourceState>,
}

/// The local tracking row of a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracking {
    pub status: LocalStatus,
    pub manual_order: i64,
    pub claimed_at: i64,
    /// When the status last changed.
    pub updated_at: i64,
}

/// A PR of the ticket, with the workspace project it belongs to (when hosted there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInfo {
    pub pr: Pr,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Checklist {
    pub total: usize,
    pub done: usize,
}

impl Checklist {
    /// At least one item, and all of them checked.
    pub fn is_complete(&self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

/// The review marker and what the branches look like now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewState {
    /// When the review was finished; `None`: not reviewed (or restarted).
    pub reviewed_at: Option<i64>,
    /// The ticket-branch tips the review covered, by project.
    pub reviewed_heads: BTreeMap<String, String>,
    /// The tips now, by project (only where the loader could read them).
    pub current_heads: BTreeMap<String, String>,
}

/// The latest recorded `uat` merge of one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeInfo {
    pub repo: String,
    pub commit: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepState {
    Ready,
    UpToDate,
    AlreadyPushed,
    Conflict,
    Blocked,
}

impl PrepState {
    pub fn as_str(self) -> &'static str {
        match self {
            PrepState::Ready => "ready",
            PrepState::UpToDate => "up_to_date",
            PrepState::AlreadyPushed => "already_pushed",
            PrepState::Conflict => "conflict",
            PrepState::Blocked => "blocked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            PrepState::Ready,
            PrepState::UpToDate,
            PrepState::AlreadyPushed,
            PrepState::Conflict,
            PrepState::Blocked,
        ]
        .into_iter()
        .find(|p| p.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepRepo {
    pub repo: String,
    pub state: PrepState,
    /// The conflicting files.
    pub files: Vec<String>,
    /// Why the repo is blocked.
    pub reason: Option<String>,
}

/// The result of the latest `integration prepare` that still describes the branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepSnapshot {
    pub at: i64,
    pub repos: Vec<PrepRepo>,
}

/// The deploy state of one touched repo (from `integration::deploy_status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployInfo {
    pub repo: String,
    /// `workspace/slug` when hosted.
    pub hosting_repo: Option<String>,
    pub state: DeployState,
    pub run_id: Option<String>,
    pub run_number: Option<u64>,
    pub run_url: Option<String>,
}

/// An unposted draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftInfo {
    pub id: i64,
    pub body: String,
    pub created_at: i64,
}

/// A cached Jira comment that mentions the author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub comment_id: String,
    pub author: String,
    pub text: String,
    pub created_at: i64,
}

/// Everything known about one ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketSnapshot {
    pub key: TicketKey,
    pub title: Option<String>,
    pub url: Option<String>,
    /// The status in Jira as last synced.
    pub jira_status: Option<String>,
    pub jira_priority: Option<String>,
    /// When the Jira row was last fetched.
    pub jira_fetched_at: Option<i64>,
    /// `None`: not claimed.
    pub tracking: Option<Tracking>,
    pub kind: TicketKind,
    /// The ticket's PRs, any state.
    pub prs: Vec<PrInfo>,
    pub checklist: Checklist,
    pub review: ReviewState,
    /// Repos the ticket branch is checked out in (from the activation records).
    pub touched_repos: Vec<String>,
    /// Repos where the test overlay is applied.
    pub overlays: Vec<String>,
    /// Touched repos with uncommitted changes.
    pub dirty_repos: Vec<String>,
    /// Latest recorded `uat` merge per repo.
    pub merges: Vec<MergeInfo>,
    /// Repos whose ticket branch has commits `uat` lacks after the recorded merge.
    pub remerge_repos: Vec<String>,
    pub prep: Option<PrepSnapshot>,
    pub deploy: Vec<DeployInfo>,
    /// The newest unposted deploy-comment draft.
    pub comment_draft: Option<DraftInfo>,
    pub comment_posted_at: Option<i64>,
    pub transition_posted_at: Option<i64>,
    /// Comments that mention the author (only gathered for Returned tickets).
    pub mentions: Vec<Mention>,
    /// The last time anything was done to the ticket (status change or audit entry).
    pub last_handled_at: Option<i64>,
    /// UAT is signed off, see [`is_testing_complete`].
    pub testing_complete: bool,
}

impl TicketSnapshot {
    /// A ticket with nothing known about it but its key.
    pub fn new(key: TicketKey) -> Self {
        Self {
            key,
            title: None,
            url: None,
            jira_status: None,
            jira_priority: None,
            jira_fetched_at: None,
            tracking: None,
            kind: TicketKind::Normal,
            prs: Vec::new(),
            checklist: Checklist::default(),
            review: ReviewState::default(),
            touched_repos: Vec::new(),
            overlays: Vec::new(),
            dirty_repos: Vec::new(),
            merges: Vec::new(),
            remerge_repos: Vec::new(),
            prep: None,
            deploy: Vec::new(),
            comment_draft: None,
            comment_posted_at: None,
            transition_posted_at: None,
            mentions: Vec::new(),
            last_handled_at: None,
            testing_complete: false,
        }
    }

    pub fn status(&self) -> Option<LocalStatus> {
        self.tracking.as_ref().map(|t| t.status)
    }

    pub fn is_tracked(&self) -> bool {
        self.tracking.is_some()
    }

    pub fn is_hotfix(&self) -> bool {
        self.kind == TicketKind::Hotfix
    }

    pub fn open_prs(&self) -> impl Iterator<Item = &PrInfo> {
        self.prs.iter().filter(|p| p.pr.state == PrState::Open)
    }

    /// `KEY "title"` or just the key.
    pub fn label(&self) -> String {
        match &self.title {
            Some(t) if !t.is_empty() => format!("{} \"{t}\"", self.key),
            _ => self.key.to_string(),
        }
    }

    pub fn manual_order(&self) -> Option<i64> {
        self.tracking.as_ref().map(|t| t.manual_order)
    }
}

/// Testing is complete once the ticket was integrated (alpha was reached) and Jira shows
/// the `uat` status or one of the `done` statuses: UAT signed off. Only then are the PRs
/// offered for approval.
pub fn is_testing_complete(
    statuses: &StatusNames,
    jira_status: Option<&str>,
    alpha_reached: bool,
) -> bool {
    alpha_reached
        && (status_is(jira_status, &statuses.uat)
            || statuses.done.iter().any(|d| status_is(jira_status, d)))
}

/// Jira priority names ranked so 0 is the most urgent; unknown or missing rank last.
pub fn jira_priority_rank(priority: Option<&str>) -> u32 {
    let Some(p) = priority else { return 5 };
    match p.trim().to_ascii_lowercase().as_str() {
        "highest" | "blocker" | "critical" => 0,
        "high" | "major" => 1,
        "medium" | "normal" => 2,
        "low" | "minor" => 3,
        "lowest" | "trivial" => 4,
        _ => 5,
    }
}

/// The whole picture the rules see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The author's Jira account id (comments mentioning it, and PRs already approved).
    pub me: Option<String>,
    pub statuses: StatusNames,
    /// Every tracked ticket in manual order, then the other cached tickets by key.
    pub tickets: Vec<TicketSnapshot>,
    pub sync: SyncSnapshot,
    pub writers: Writers,
}

impl Snapshot {
    pub fn ticket(&self, key: &TicketKey) -> Option<&TicketSnapshot> {
        self.tickets.iter().find(|t| &t.key == key)
    }

    /// The one `Active` ticket.
    pub fn active(&self) -> Option<&TicketSnapshot> {
        self.tickets
            .iter()
            .find(|t| t.status() == Some(LocalStatus::Active))
    }
}

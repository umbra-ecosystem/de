//! The simulation's own data: the shapes the real core serves (tickets, PRs, runs). Never seen by a view.

use std::collections::{BTreeMap, BTreeSet};

use crate::vm::Block;

pub const ME: &str = "You";
pub const JIRA_REVIEW: &str = "In Review";
pub const JIRA_ALPHA: &str = "Alpha Testing";
pub const JIRA_RETURNED: &str = "Returned";
pub const JIRA_SIGNED: [&str; 1] = ["Done"];

/// One simulated minute takes four real seconds.
pub const MS_PER_MIN: i64 = 4000;
pub const START_MIN: i64 = 9 * 60 + 40;

#[derive(Clone, Debug)]
pub struct RepoCfg {
    pub name: &'static str,
    pub base: &'static str,
    pub prod: &'static str,
    pub host: &'static str,
    pub services: u32,
    pub overlay_consumer: bool,
    pub consumes: Option<&'static str>,
    pub checks: &'static [&'static str],
}

pub fn repos() -> Vec<RepoCfg> {
    vec![
        RepoCfg {
            name: "api-client",
            base: "develop",
            prod: "master",
            host: "acme/api-client",
            services: 3,
            overlay_consumer: false,
            consumes: None,
            checks: &["Unit tests pass", "Lint clean"],
        },
        RepoCfg {
            name: "web",
            base: "develop",
            prod: "main",
            host: "acme/web",
            services: 2,
            overlay_consumer: true,
            consumes: Some("api-client"),
            checks: &["Unit tests pass", "Build succeeds"],
        },
        RepoCfg {
            name: "worker",
            base: "develop",
            prod: "master",
            host: "acme/worker",
            services: 1,
            overlay_consumer: false,
            consumes: None,
            checks: &["Unit tests pass"],
        },
        RepoCfg {
            name: "docs",
            base: "master",
            prod: "master",
            host: "acme/docs",
            services: 0,
            overlay_consumer: false,
            consumes: None,
            checks: &[],
        },
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Highest,
    High,
    Medium,
    Low,
}

impl Priority {
    pub fn rank(self) -> u8 {
        match self {
            Priority::Highest => 0,
            Priority::High => 1,
            Priority::Medium => 2,
            Priority::Low => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Priority::Highest => "Highest",
            Priority::High => "High",
            Priority::Medium => "Medium",
            Priority::Low => "Low",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Local {
    Claimed,
    Reviewing,
    Active,
    Parked,
    Integrated,
    Done,
}

impl Local {
    pub fn label(self) -> &'static str {
        match self {
            Local::Claimed => "Claimed",
            Local::Reviewing => "Reviewing",
            Local::Active => "Active",
            Local::Parked => "Parked",
            Local::Integrated => "Integrated",
            Local::Done => "Done",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Local::Claimed => "claimed",
            Local::Reviewing => "reviewing",
            Local::Active => "active",
            Local::Parked => "parked",
            Local::Integrated => "integrated",
            Local::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Local {
        match s {
            "reviewing" => Local::Reviewing,
            "active" => Local::Active,
            "parked" => Local::Parked,
            "integrated" => Local::Integrated,
            "done" => Local::Done,
            _ => Local::Claimed,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Comment {
    pub n: u32,
    pub who: String,
    pub at: String,
    pub body: Vec<Block>,
}

#[derive(Clone, Debug)]
pub struct IssueLink {
    pub rel: &'static str,
    pub key: &'static str,
    pub title: &'static str,
    pub status: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ctx,
    Add,
    Del,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub k: Kind,
    pub o: Option<u32>,
    pub n: Option<u32>,
    pub x: String,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: String,
    pub adds: u32,
    pub dels: u32,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug)]
pub struct Thread {
    pub id: String,
    pub file: String,
    pub line: Option<String>,
    pub author: String,
    pub text: String,
    pub resolved: bool,
}

#[derive(Clone, Debug)]
pub struct Reviewer {
    pub name: String,
    pub approved: bool,
}

#[derive(Clone, Debug)]
pub struct SinceReview {
    pub commit: String,
    pub msg: String,
    pub files: Vec<FileDiff>,
}

#[derive(Clone, Debug)]
pub struct Pr {
    pub repo: String,
    pub id: u32,
    pub title: String,
    pub src: String,
    pub dst: String,
    pub updated_seq: u32,
    pub reviewers: Vec<Reviewer>,
    pub files: Vec<FileDiff>,
    pub threads: Vec<Thread>,
    pub since: Option<SinceReview>,
}

#[derive(Clone, Debug)]
pub struct Merge {
    pub repo: String,
    pub commit: String,
    pub at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployState {
    Pending,
    Running,
    Deployed,
    Failed,
}

impl DeployState {
    pub fn word(self) -> &'static str {
        match self {
            DeployState::Pending => "pending",
            DeployState::Running => "running",
            DeployState::Deployed => "deployed",
            DeployState::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct UatMoved {
    pub by: String,
    pub ticket: String,
    pub at: String,
}

#[derive(Clone, Debug)]
pub struct Deploy {
    pub state: DeployState,
    pub run: u32,
    pub since: i64,
    pub step: String,
    pub log: Option<Vec<String>>,
    pub uat_moved: Option<UatMoved>,
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub id: String,
    pub body: String,
    pub posted: bool,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub repo: String,
    pub ticket_role: bool,
    pub branch: String,
    pub prev: String,
    pub stash: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Overlay {
    pub repo: String,
    pub provider: String,
}

#[derive(Clone, Debug)]
pub struct Activation {
    pub started_ms: i64,
    pub records: Vec<Record>,
    pub overlay: Option<Overlay>,
}

#[derive(Clone, Debug)]
pub enum PrepOutcome {
    Ready {
        branch: String,
        commits: u32,
        files: u32,
        uat_before: String,
        merge: String,
        overlaps: Vec<(String, String)>,
    },
    AlreadyPushed {
        commit: String,
    },
    Conflict {
        files: Vec<String>,
        with: String,
    },
    Blocked {
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub struct PrepItem {
    pub repo: String,
    pub outcome: PrepOutcome,
}

#[derive(Clone, Debug)]
pub struct Prep {
    pub at: i64,
    pub rows: Vec<PrepItem>,
}

#[derive(Clone, Copy, Debug)]
pub struct Wait {
    pub since: i64,
    pub mins: u32,
}

#[derive(Clone, Debug)]
pub struct Ticket {
    pub key: String,
    pub kind: &'static str,
    pub priority: Priority,
    pub title: String,
    pub jira: String,
    pub assignee: &'static str,
    pub reporter: &'static str,
    pub sprint: &'static str,
    pub epic: &'static str,
    pub fix_version: &'static str,
    pub estimate: &'static str,
    pub created: &'static str,
    pub updated: &'static str,
    pub labels: Vec<&'static str>,
    pub components: Vec<&'static str>,
    pub desc: Vec<Block>,
    pub ac: Vec<String>,
    pub comments: Vec<Comment>,
    pub attachments: Vec<(&'static str, &'static str)>,
    pub links: Vec<IssueLink>,
    pub subtasks: Vec<(bool, &'static str)>,
    pub cands: BTreeMap<String, Vec<String>>,
    pub local: Option<Local>,
    pub reviewed: bool,
    pub reviewed_seq: Option<u32>,
    pub checklist: Vec<(String, bool)>,
    pub notes: String,
    pub act: Option<Activation>,
    pub time_ms: i64,
    pub merges: Vec<Merge>,
    pub deploy: BTreeMap<String, Deploy>,
    pub drafts: Vec<Draft>,
    pub prep: Option<Prep>,
    pub pre: BTreeSet<String>,
    pub link: BTreeMap<String, String>,
    pub excl: BTreeSet<String>,
    pub seen_n: u32,
    pub new_commits: bool,
    pub prs: Vec<Pr>,
    pub pr_wait: Option<Wait>,
    pub th_wait: Option<Wait>,
    pub accepted: Vec<String>,
    pub conflict_sent: Option<(String, String)>,
    pub test_from_n: Option<u32>,
}

impl Ticket {
    pub fn blank(key: &str, title: &str) -> Self {
        Self {
            key: key.to_string(),
            kind: "Task",
            priority: Priority::Medium,
            title: title.to_string(),
            jira: JIRA_REVIEW.to_string(),
            assignee: "",
            reporter: "",
            sprint: "Sprint 41",
            epic: "",
            fix_version: "2.14.0",
            estimate: "1 pt",
            created: "today",
            updated: "just now",
            labels: Vec::new(),
            components: Vec::new(),
            desc: Vec::new(),
            ac: Vec::new(),
            comments: Vec::new(),
            attachments: Vec::new(),
            links: Vec::new(),
            subtasks: Vec::new(),
            cands: BTreeMap::new(),
            local: None,
            reviewed: false,
            reviewed_seq: None,
            checklist: Vec::new(),
            notes: String::new(),
            act: None,
            time_ms: 0,
            merges: Vec::new(),
            deploy: BTreeMap::new(),
            drafts: Vec::new(),
            prep: None,
            pre: BTreeSet::new(),
            link: BTreeMap::new(),
            excl: BTreeSet::new(),
            seen_n: 0,
            new_commits: false,
            prs: Vec::new(),
            pr_wait: None,
            th_wait: None,
            accepted: Vec::new(),
            conflict_sent: None,
            test_from_n: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub id: u64,
    pub at: String,
    pub action: String,
    pub ticket: Option<String>,
    pub repo: Option<String>,
    pub outcome: &'static str,
    pub details: String,
}

#[derive(Clone, Debug)]
pub enum LockKind {
    External,
    Stale,
    Sync,
}

#[derive(Clone, Debug)]
pub struct Lock {
    pub by: String,
    pub kind: LockKind,
    pub op: String,
}

#[derive(Clone, Debug)]
pub enum Response {
    Dismissed { hash: String },
    Snoozed { until: i64 },
}

#[derive(Clone, Debug, Default)]
pub struct SyncState {
    pub running: bool,
    pub last_ok: i64,
    pub last_attempt: i64,
    pub last_error: Option<String>,
    pub report: Vec<(bool, String, String)>,
    pub script: usize,
    pub finish_at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Sims {
    pub offline: bool,
    pub conflict: Option<String>,
    pub leak: Option<String>,
    pub uat_moved: bool,
    pub fail_deploy: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub branches: BTreeMap<String, String>,
    pub dirty: BTreeSet<String>,
    pub up: bool,
}

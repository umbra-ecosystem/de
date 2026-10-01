//! The simulation's own data: the shapes the real core serves (tickets, PRs, runs). Never seen by a view.
//!
//! A ticket's local lifecycle is one [`Stage`] enum. What only exists in some stages (the activation, the
//! integration preparation, the landed merges) lives inside those variants, and every change of stage goes
//! through a method of `Stage` that refuses an illegal move.

use std::collections::{BTreeMap, BTreeSet};

use crate::vm::{
    Block, Branch, DraftId, LineAnchor, Phase, PrNumber, RepoName, SuggestionId, TicketKey,
};

pub const ME: &str = "You";

/// One simulated minute takes four real seconds.
pub const MS_PER_MIN: i64 = 4000;
pub const START_MIN: i64 = 9 * 60 + 40;

/* ---------------------------- Jira ---------------------------- */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JiraStatus {
    InReview,
    AlphaTesting,
    Returned,
    Done,
    /// A workflow status the prototype has no word for; the ticket's `jira_name` says which.
    Other,
}

impl JiraStatus {
    pub fn label(self) -> &'static str {
        match self {
            JiraStatus::InReview => "In Review",
            JiraStatus::AlphaTesting => "Alpha Testing",
            JiraStatus::Returned => "Returned",
            JiraStatus::Done => "Done",
            JiraStatus::Other => "Jira",
        }
    }

    /// The configured "signed off" statuses (`signed_off` in the real config; default the Done ones).
    pub fn is_signed_off(self) -> bool {
        matches!(self, JiraStatus::Done)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Highest,
    High,
    Medium,
    Low,
}

impl Priority {
    pub fn label(self) -> &'static str {
        match self {
            Priority::Highest => "Highest",
            Priority::High => "High",
            Priority::Medium => "Medium",
            Priority::Low => "Low",
        }
    }
}

// Variant order is the claim-queue order.
impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Priority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |p: &Priority| match p {
            Priority::Highest => 0,
            Priority::High => 1,
            Priority::Medium => 2,
            Priority::Low => 3,
        };
        rank(self).cmp(&rank(other))
    }
}

/* ---------------------------- repos ---------------------------- */

#[derive(Clone, Debug)]
pub struct RepoCfg {
    pub name: RepoName,
    pub base: Branch,
    pub prod: Branch,
    pub host: &'static str,
    pub services: u32,
    pub overlay_consumer: bool,
    pub consumes: Option<RepoName>,
    pub checks: &'static [&'static str],
}

pub fn repos() -> Vec<RepoCfg> {
    let cfg = |name: &str,
               base: &str,
               prod: &str,
               host: &'static str,
               services: u32,
               consumes: Option<&str>,
               checks: &'static [&'static str]| RepoCfg {
        name: name.into(),
        base: base.into(),
        prod: prod.into(),
        host,
        services,
        overlay_consumer: consumes.is_some(),
        consumes: consumes.map(RepoName::from),
        checks,
    };
    vec![
        cfg(
            "api-client",
            "develop",
            "master",
            "acme/api-client",
            3,
            None,
            &["Unit tests pass", "Lint clean"],
        ),
        cfg(
            "web",
            "develop",
            "main",
            "acme/web",
            2,
            Some("api-client"),
            &["Unit tests pass", "Build succeeds"],
        ),
        cfg(
            "worker",
            "develop",
            "master",
            "acme/worker",
            1,
            None,
            &["Unit tests pass"],
        ),
        cfg("docs", "master", "master", "acme/docs", 0, None, &[]),
    ]
}

/* ---------------------------- the ticket's lifecycle ---------------------------- */

/// Where a ticket is in your hands. Derived from [`Stage`]; never stored.
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
}

/// The review covered the pull requests as of this update sequence number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewMark(pub u32);

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
    pub ticket: TicketKey,
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

/// A merge of the ticket that was pushed to `uat`, with the Actions run it started. One value, so a merge
/// cannot exist without its run.
#[derive(Clone, Debug)]
pub struct Landing {
    pub repo: RepoName,
    pub commit: String,
    pub at: String,
    pub deploy: Deploy,
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub id: DraftId,
    pub body: String,
    pub posted: bool,
}

/// What a ticket carries once something has been pushed. Empty until then.
#[derive(Clone, Debug, Default)]
pub struct Shipped {
    pub landings: Vec<Landing>,
    pub drafts: Vec<Draft>,
}

/// What every stage after claiming shares: the review mark and what was shipped.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub review: Option<ReviewMark>,
    pub shipped: Shipped,
}

#[derive(Clone, Debug)]
pub struct Record {
    pub repo: RepoName,
    pub ticket_role: bool,
    pub branch: Branch,
    pub prev: Branch,
    pub stash: bool,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Overlay {
    pub repo: RepoName,
    pub provider: RepoName,
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
        branch: Branch,
        commits: u32,
        files: u32,
        uat_before: String,
        merge: String,
        overlaps: Vec<(TicketKey, String)>,
    },
    AlreadyPushed {
        commit: String,
    },
    Conflict {
        files: Vec<String>,
        with: TicketKey,
    },
    Blocked {
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub struct PrepItem {
    pub repo: RepoName,
    pub outcome: PrepOutcome,
}

#[derive(Clone, Debug)]
pub struct Prep {
    pub at: i64,
    pub rows: Vec<PrepItem>,
}

#[derive(Clone, Debug)]
pub enum Stage {
    /// Only in Jira's Review column (or returned): not yours.
    Unclaimed,
    /// Claimed, in review, or parked.
    InHand {
        phase: Phase,
        held: Held,
    },
    /// The repos are on its branches. Only here do the activation and the integration preparation exist.
    Active {
        held: Held,
        act: Activation,
        prep: Option<Prep>,
    },
    /// Pushed to `uat`; waiting for alpha.
    Integrated {
        held: Held,
    },
    Done {
        held: Held,
    },
}

/// A refused move: what was attempted, from which stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Illegal {
    pub action: &'static str,
    pub from: &'static str,
}

impl std::fmt::Display for Illegal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot {} a ticket that is {}", self.action, self.from)
    }
}

/// Where an active ticket goes when its repos are restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Park,
    Integrate,
}

impl Stage {
    pub fn name(&self) -> &'static str {
        match self {
            Stage::Unclaimed => "unclaimed",
            Stage::InHand { phase, .. } => match phase {
                Phase::Claimed => "claimed",
                Phase::Reviewing => "reviewing",
                Phase::Parked => "parked",
            },
            Stage::Active { .. } => "active",
            Stage::Integrated { .. } => "integrated",
            Stage::Done { .. } => "done",
        }
    }

    pub fn local(&self) -> Option<Local> {
        Some(match self {
            Stage::Unclaimed => return None,
            Stage::InHand { phase, .. } => match phase {
                Phase::Claimed => Local::Claimed,
                Phase::Reviewing => Local::Reviewing,
                Phase::Parked => Local::Parked,
            },
            Stage::Active { .. } => Local::Active,
            Stage::Integrated { .. } => Local::Integrated,
            Stage::Done { .. } => Local::Done,
        })
    }

    pub fn held(&self) -> Option<&Held> {
        match self {
            Stage::Unclaimed => None,
            Stage::InHand { held, .. }
            | Stage::Active { held, .. }
            | Stage::Integrated { held }
            | Stage::Done { held } => Some(held),
        }
    }

    pub fn held_mut(&mut self) -> Option<&mut Held> {
        match self {
            Stage::Unclaimed => None,
            Stage::InHand { held, .. }
            | Stage::Active { held, .. }
            | Stage::Integrated { held }
            | Stage::Done { held } => Some(held),
        }
    }

    /// Run a transition in place. `f` gets the current stage and returns the next one, or hands the
    /// unchanged stage back (`Err`) when the move is not in the table; the stage is then left as it was.
    fn apply(
        &mut self,
        action: &'static str,
        f: impl FnOnce(Stage) -> Result<Stage, Stage>,
    ) -> Result<(), Illegal> {
        let current = std::mem::replace(self, Stage::Unclaimed);
        match f(current) {
            Ok(next) => {
                *self = next;
                Ok(())
            }
            Err(unchanged) => {
                let from = unchanged.name();
                *self = unchanged;
                Err(Illegal { action, from })
            }
        }
    }

    /* ---- the transition table: every change of stage is one of these ---- */

    /// Unclaimed -> Claimed.
    pub fn claim(&mut self) -> Result<(), Illegal> {
        self.apply("claim", |s| match s {
            Stage::Unclaimed => Ok(Stage::InHand {
                phase: Phase::Claimed,
                held: Held::default(),
            }),
            s => Err(s),
        })
    }

    /// Claimed -> Unclaimed (the undo of a claim).
    pub fn unclaim(&mut self) -> Result<(), Illegal> {
        self.apply("unclaim", |s| match s {
            Stage::InHand {
                phase: Phase::Claimed,
                ..
            } => Ok(Stage::Unclaimed),
            s => Err(s),
        })
    }

    /// Unclaimed, Claimed, Reviewing -> Reviewing; Parked stays parked. Clears the review mark.
    pub fn start_review(&mut self) -> Result<(), Illegal> {
        self.apply("start a review of", |s| match s {
            Stage::Unclaimed => Ok(Stage::InHand {
                phase: Phase::Reviewing,
                held: Held::default(),
            }),
            Stage::InHand { phase, mut held } => {
                held.review = None;
                Ok(Stage::InHand {
                    phase: if phase == Phase::Parked {
                        Phase::Parked
                    } else {
                        Phase::Reviewing
                    },
                    held,
                })
            }
            s => Err(s),
        })
    }

    /// In hand -> marked reviewed (a claimed ticket becomes Reviewing; a parked one stays parked).
    pub fn mark_reviewed(&mut self, mark: ReviewMark) -> Result<(), Illegal> {
        self.apply("mark as reviewed", |s| match s {
            Stage::InHand { phase, mut held } => {
                held.review = Some(mark);
                Ok(Stage::InHand {
                    phase: if phase == Phase::Claimed {
                        Phase::Reviewing
                    } else {
                        phase
                    },
                    held,
                })
            }
            s => Err(s),
        })
    }

    /// The undo of `mark_reviewed`: back to the recorded phase and mark.
    pub fn restore_review(
        &mut self,
        phase: Phase,
        mark: Option<ReviewMark>,
    ) -> Result<(), Illegal> {
        self.apply("restore the review of", |s| match s {
            Stage::InHand { mut held, .. } => {
                held.review = mark;
                Ok(Stage::InHand { phase, held })
            }
            s => Err(s),
        })
    }

    /// Claimed, Reviewing, Parked, Integrated -> Active. The preparation starts empty.
    pub fn activate(&mut self, act: Activation) -> Result<(), Illegal> {
        self.apply("activate", |s| match s {
            Stage::InHand { held, .. } | Stage::Integrated { held } => Ok(Stage::Active {
                held,
                act,
                prep: None,
            }),
            s => Err(s),
        })
    }

    /// Active -> Parked or Integrated, handing back the activation so the repos can be restored.
    pub fn finish_active(&mut self, after: After) -> Result<Activation, Illegal> {
        let mut taken = None;
        self.apply("restore", |s| match s {
            Stage::Active { held, act, .. } => {
                taken = Some(act);
                Ok(match after {
                    After::Park => Stage::InHand {
                        phase: Phase::Parked,
                        held,
                    },
                    After::Integrate => Stage::Integrated { held },
                })
            }
            s => Err(s),
        })?;
        Ok(taken.expect("set by the Ok arm"))
    }

    /// Claimed or Reviewing -> Parked (to take another ticket). Active ones go through `finish_active`.
    pub fn park_in_hand(&mut self) -> Result<(), Illegal> {
        self.apply("park", |s| match s {
            Stage::InHand { held, .. } => Ok(Stage::InHand {
                phase: Phase::Parked,
                held,
            }),
            s => Err(s),
        })
    }

    /// Integrated or Done -> Claimed again, treated as new (what was shipped is forgotten).
    pub fn reclaim(&mut self) -> Result<(), Illegal> {
        self.apply("claim again", |s| match s {
            Stage::Integrated { .. } | Stage::Done { .. } => Ok(Stage::InHand {
                phase: Phase::Claimed,
                held: Held::default(),
            }),
            s => Err(s),
        })
    }

    /// The ticket was returned to development in Jira. One that was never shipped goes back to Unclaimed
    /// (it comes back as new); one that had shipped (a re-merge that was returned, or parked) goes back to
    /// Integrated with its landings, drafts and review mark intact. An active ticket must be restored first.
    pub fn returned(&mut self) -> Result<(), Illegal> {
        self.apply("return", |s| match s {
            Stage::InHand { held, .. } => {
                if held.shipped.landings.is_empty() && held.shipped.drafts.is_empty() {
                    Ok(Stage::Unclaimed)
                } else {
                    Ok(Stage::Integrated { held })
                }
            }
            s => Err(s),
        })
    }

    /// Integrated -> Done, once every pull request is approved.
    pub fn approved(&mut self) -> Result<(), Illegal> {
        self.apply("finish", |s| match s {
            Stage::Integrated { held } => Ok(Stage::Done { held }),
            s => Err(s),
        })
    }
}

/* ---------------------------- ticket ---------------------------- */

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
    pub key: TicketKey,
    pub title: &'static str,
    pub status: JiraStatus,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ThreadId(pub u64);

#[derive(Clone, Debug)]
pub struct Thread {
    pub id: ThreadId,
    pub file: String,
    pub line: Option<LineAnchor>,
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
    pub repo: RepoName,
    pub id: PrNumber,
    pub title: String,
    pub src: Branch,
    pub dst: Branch,
    pub updated_seq: u32,
    pub reviewers: Vec<Reviewer>,
    pub files: Vec<FileDiff>,
    pub threads: Vec<Thread>,
    pub since: Option<SinceReview>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wait {
    pub since: i64,
    pub mins: u32,
}

#[derive(Clone, Debug)]
pub struct Ticket {
    pub key: TicketKey,
    pub kind: &'static str,
    pub priority: Priority,
    pub title: String,
    pub jira: JiraStatus,
    /// Jira's own spelling of the status, when it is not one of the four above.
    pub jira_name: Option<String>,
    pub assignee: &'static str,
    pub reporter: &'static str,
    pub sprint: &'static str,
    pub epic: &'static str,
    pub fix_version: &'static str,
    pub estimate: &'static str,
    pub created: &'static str,
    pub updated: &'static str,
    /// When Jira last changed the ticket (unix seconds), when known: what the "2d ago" is worked out from.
    pub updated_at: Option<i64>,
    /// When the store first saw the ticket in its current Jira status (unix seconds), when it keeps that: a wait
    /// for a missing pull request is measured from it. Without it the wait runs from when it is first noticed.
    pub status_since: Option<i64>,
    pub labels: Vec<&'static str>,
    pub components: Vec<&'static str>,
    pub desc: Vec<Block>,
    pub ac: Vec<String>,
    pub comments: Vec<Comment>,
    pub attachments: Vec<(&'static str, &'static str)>,
    pub links: Vec<IssueLink>,
    pub subtasks: Vec<(bool, &'static str)>,
    pub cands: BTreeMap<RepoName, Vec<Branch>>,
    pub stage: Stage,
    pub checklist: Vec<(String, bool)>,
    pub notes: String,
    pub time_ms: i64,
    /// Pre-push checks ticked: `(repo, index)`.
    pub pre: BTreeSet<(RepoName, usize)>,
    pub link: BTreeMap<RepoName, Branch>,
    pub excl: BTreeSet<RepoName>,
    pub seen_n: u32,
    pub new_commits: bool,
    pub prs: Vec<Pr>,
    pub pr_wait: Option<Wait>,
    pub th_wait: Option<Wait>,
    pub accepted: Vec<ThreadId>,
    pub conflict_sent: Option<(String, String)>,
    pub test_from_n: Option<u32>,
}

impl Ticket {
    pub fn blank(key: &str, title: &str) -> Self {
        Self {
            key: key.into(),
            kind: "Task",
            priority: Priority::Medium,
            title: title.to_string(),
            jira: JiraStatus::InReview,
            jira_name: None,
            assignee: "",
            reporter: "",
            sprint: "Sprint 41",
            epic: "",
            fix_version: "2.14.0",
            estimate: "1 pt",
            created: "today",
            updated: "just now",
            updated_at: None,
            status_since: None,
            labels: Vec::new(),
            components: Vec::new(),
            desc: Vec::new(),
            ac: Vec::new(),
            comments: Vec::new(),
            attachments: Vec::new(),
            links: Vec::new(),
            subtasks: Vec::new(),
            cands: BTreeMap::new(),
            stage: Stage::Unclaimed,
            checklist: Vec::new(),
            notes: String::new(),
            time_ms: 0,
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

    /* ---- read access that hides the stage shapes ---- */

    pub fn local(&self) -> Option<Local> {
        self.stage.local()
    }

    pub fn reviewed(&self) -> bool {
        self.review_mark().is_some()
    }

    pub fn review_mark(&self) -> Option<ReviewMark> {
        self.stage.held().and_then(|h| h.review)
    }

    pub fn act(&self) -> Option<&Activation> {
        match &self.stage {
            Stage::Active { act, .. } => Some(act),
            _ => None,
        }
    }

    pub fn prep(&self) -> Option<&Prep> {
        match &self.stage {
            Stage::Active { prep, .. } => prep.as_ref(),
            _ => None,
        }
    }

    pub fn landings(&self) -> &[Landing] {
        self.stage.held().map_or(&[], |h| &h.shipped.landings)
    }

    pub fn landing(&self, repo: &RepoName) -> Option<&Landing> {
        self.landings().iter().find(|l| l.repo == *repo)
    }

    pub fn landing_mut(&mut self, repo: &RepoName) -> Option<&mut Landing> {
        self.stage
            .held_mut()?
            .shipped
            .landings
            .iter_mut()
            .find(|l| l.repo == *repo)
    }

    pub fn draft_mut(&mut self, id: &DraftId) -> Option<&mut Draft> {
        self.stage
            .held_mut()?
            .shipped
            .drafts
            .iter_mut()
            .find(|d| d.id == *id)
    }

    pub fn has_landed(&self, repo: &RepoName) -> bool {
        self.landing(repo).is_some()
    }

    pub fn drafts(&self) -> &[Draft] {
        self.stage.held().map_or(&[], |h| &h.shipped.drafts)
    }
}

/* ---------------------------- world ---------------------------- */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    Success,
    Failure,
    Skipped,
}

impl AuditOutcome {
    pub fn word(self) -> &'static str {
        match self {
            AuditOutcome::Success => "success",
            AuditOutcome::Failure => "failure",
            AuditOutcome::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuditEntry {
    pub id: u64,
    pub at: String,
    pub action: String,
    pub ticket: Option<TicketKey>,
    pub repo: Option<RepoName>,
    pub outcome: AuditOutcome,
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

pub type Responses = BTreeMap<SuggestionId, Response>;

#[derive(Clone, Debug, Default)]
pub struct SyncState {
    pub running: bool,
    pub last_ok: i64,
    pub last_attempt: i64,
    pub last_error: Option<String>,
    pub report: Vec<(bool, String, String)>,
    pub script: usize,
    pub finish_at: Option<i64>,
    /// No successful sync is on record (a real store that has never synced).
    pub never: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Sims {
    pub offline: bool,
    pub conflict: Option<RepoName>,
    pub leak: Option<RepoName>,
    pub uat_moved: bool,
    pub fail_deploy: Option<RepoName>,
    /// Docker is not running: starting or stopping services fails.
    pub docker_down: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    pub branches: BTreeMap<RepoName, Branch>,
    pub dirty: BTreeSet<RepoName>,
    pub up: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act() -> Activation {
        Activation {
            started_ms: 0,
            records: Vec::new(),
            overlay: None,
        }
    }

    fn stage(name: &str) -> Stage {
        let held = Held::default();
        match name {
            "unclaimed" => Stage::Unclaimed,
            "claimed" => Stage::InHand {
                phase: Phase::Claimed,
                held,
            },
            "reviewing" => Stage::InHand {
                phase: Phase::Reviewing,
                held,
            },
            "parked" => Stage::InHand {
                phase: Phase::Parked,
                held,
            },
            "active" => Stage::Active {
                held,
                act: act(),
                prep: None,
            },
            "integrated" => Stage::Integrated { held },
            "done" => Stage::Done { held },
            other => panic!("{other}"),
        }
    }

    const ALL: [&str; 7] = [
        "unclaimed",
        "claimed",
        "reviewing",
        "parked",
        "active",
        "integrated",
        "done",
    ];

    /// `(action, stages it is allowed from, stage it leads to)`. Anything not listed must be refused
    /// and must leave the stage untouched.
    type Step = fn(&mut Stage) -> bool;
    type Row = (&'static str, Step, Vec<(&'static str, &'static str)>);
    fn table() -> Vec<Row> {
        vec![
            (
                "claim",
                |s| s.claim().is_ok(),
                vec![("unclaimed", "claimed")],
            ),
            (
                "unclaim",
                |s| s.unclaim().is_ok(),
                vec![("claimed", "unclaimed")],
            ),
            (
                "start_review",
                |s| s.start_review().is_ok(),
                vec![
                    ("unclaimed", "reviewing"),
                    ("claimed", "reviewing"),
                    ("reviewing", "reviewing"),
                    ("parked", "parked"),
                ],
            ),
            (
                "mark_reviewed",
                |s| s.mark_reviewed(ReviewMark(1)).is_ok(),
                vec![
                    ("claimed", "reviewing"),
                    ("reviewing", "reviewing"),
                    ("parked", "parked"),
                ],
            ),
            (
                "activate",
                |s| s.activate(act()).is_ok(),
                vec![
                    ("claimed", "active"),
                    ("reviewing", "active"),
                    ("parked", "active"),
                    ("integrated", "active"),
                ],
            ),
            (
                "park_in_hand",
                |s| s.park_in_hand().is_ok(),
                vec![
                    ("claimed", "parked"),
                    ("reviewing", "parked"),
                    ("parked", "parked"),
                ],
            ),
            (
                "reclaim",
                |s| s.reclaim().is_ok(),
                vec![("integrated", "claimed"), ("done", "claimed")],
            ),
            (
                "returned",
                |s| s.returned().is_ok(),
                vec![
                    ("claimed", "unclaimed"),
                    ("reviewing", "unclaimed"),
                    ("parked", "unclaimed"),
                ],
            ),
            (
                "approved",
                |s| s.approved().is_ok(),
                vec![("integrated", "done")],
            ),
            (
                "finish_active(park)",
                |s| s.finish_active(After::Park).is_ok(),
                vec![("active", "parked")],
            ),
            (
                "finish_active(integrate)",
                |s| s.finish_active(After::Integrate).is_ok(),
                vec![("active", "integrated")],
            ),
        ]
    }

    #[test]
    fn every_transition_is_allowed_only_where_the_table_says() {
        for (name, step, allowed) in table() {
            for from in ALL {
                let mut s = stage(from);
                let ok = step(&mut s);
                match allowed.iter().find(|(f, _)| *f == from) {
                    Some((_, to)) => {
                        assert!(ok, "{name} should be allowed from {from}");
                        assert_eq!(s.name(), *to, "{name} from {from}");
                    }
                    None => {
                        assert!(!ok, "{name} must be refused from {from}");
                        assert_eq!(
                            s.name(),
                            from,
                            "{name} refused from {from} must not move it"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_review_mark_survives_parking_and_is_cleared_by_starting_over() {
        let mut s = stage("claimed");
        s.mark_reviewed(ReviewMark(3)).unwrap();
        s.park_in_hand().unwrap();
        assert_eq!(s.held().unwrap().review, Some(ReviewMark(3)));
        s.start_review().unwrap();
        assert_eq!(s.held().unwrap().review, None);
    }

    #[test]
    fn what_was_shipped_survives_a_remerge_but_not_a_reclaim() {
        let mut s = Stage::Integrated {
            held: Held {
                review: Some(ReviewMark(1)),
                shipped: Shipped {
                    landings: vec![Landing {
                        repo: "web".into(),
                        commit: "abc".into(),
                        at: "now".into(),
                        deploy: Deploy {
                            state: DeployState::Deployed,
                            run: 1,
                            since: 0,
                            step: String::new(),
                            log: None,
                            uat_moved: None,
                        },
                    }],
                    drafts: Vec::new(),
                },
            },
        };
        s.activate(act()).unwrap();
        assert_eq!(
            s.held().unwrap().shipped.landings.len(),
            1,
            "re-merge keeps the landings"
        );
        s.finish_active(After::Integrate).unwrap();
        s.reclaim().unwrap();
        assert!(s.held().unwrap().shipped.landings.is_empty());
    }

    #[test]
    fn returning_a_ticket_that_had_shipped_keeps_what_it_shipped() {
        let landing = Landing {
            repo: "web".into(),
            commit: "abc".into(),
            at: "now".into(),
            deploy: Deploy {
                state: DeployState::Deployed,
                run: 1,
                since: 0,
                step: String::new(),
                log: None,
                uat_moved: None,
            },
        };
        let mut s = Stage::Integrated {
            held: Held {
                review: Some(ReviewMark(1)),
                shipped: Shipped {
                    landings: vec![landing],
                    drafts: Vec::new(),
                },
            },
        };
        s.activate(act()).unwrap();
        s.finish_active(After::Park).unwrap();
        s.returned().unwrap();
        assert_eq!(s.name(), "integrated");
        let held = s.held().unwrap();
        assert_eq!(held.shipped.landings.len(), 1);
        assert_eq!(held.review, Some(ReviewMark(1)));
    }

    #[test]
    fn an_activation_and_a_preparation_exist_only_while_active() {
        let mut t = Ticket::blank("PROJ-1", "x");
        t.stage = stage("claimed");
        assert!(t.act().is_none() && t.prep().is_none());
        t.stage.activate(act()).unwrap();
        assert!(t.act().is_some());
        t.stage.finish_active(After::Park).unwrap();
        assert!(t.act().is_none());
    }
}

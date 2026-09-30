//! One view-model type per screen area. The store builds them; the views draw them.

use super::common::*;
use super::ids::*;
use super::intent::{Command, DiffMode, Intent};

/* ------------------------------ shell ------------------------------ */

#[derive(Clone, Debug, PartialEq)]
pub struct Counts {
    pub groups: Vec<(Group, usize)>,
    /// Suggestions that are actionable right now (the sidebar badge on Next).
    pub suggestions: usize,
    pub attention: usize,
}

impl Counts {
    pub fn group(&self, g: Group) -> usize {
        self.groups
            .iter()
            .find(|(x, _)| *x == g)
            .map_or(0, |(_, n)| *n)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabInfo {
    pub title: String,
    pub bad: bool,
    pub unseen: bool,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveChip {
    pub key: TicketKey,
    pub elapsed: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StatusVm {
    pub active: Option<ActiveChip>,
    pub overlay: Option<RepoName>,
    pub chips: Vec<Badge>,
    pub jira_ready: bool,
    pub gh_ready: bool,
    pub sync_text: String,
    pub sync_tone: Tone,
    pub sync_running: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RightRow {
    Kv(Kv),
    Text(String),
    Muted(String),
    Item {
        head: Vec<Badge>,
        key: Option<TicketKey>,
        title: String,
        sub: Option<String>,
    },
    Activity {
        at: String,
        text: String,
        failed: bool,
    },
    Report {
        ok: bool,
        source: String,
        text: String,
    },
    Button(Btn),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RightSection {
    pub title: String,
    pub rows: Vec<RightRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteItem {
    pub label: String,
    pub hint: String,
    pub intent: Intent,
}

/* ------------------------------ next ------------------------------ */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Automatic,
    Local,
    External,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SugState {
    Open,
    Resurfaced,
    Dismissed,
    Snoozed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionCard {
    pub id: SuggestionId,
    /// Needs you now (something broke, someone waits, a hotfix): the one thing the UI draws in bold.
    pub attention: bool,
    pub rank: usize,
    pub hotfix: bool,
    pub info: bool,
    pub ticket: Option<TicketKey>,
    pub title: String,
    pub reason: String,
    pub level: Level,
    pub state: SugState,
    pub primary: Btn,
    pub alt: Option<Btn>,
    pub dismiss: Btn,
    pub snooze: Vec<Btn>,
    pub undo: Btn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NextVm {
    pub cards: Vec<SuggestionCard>,
    pub hidden: usize,
    pub show_all: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttentionVm {
    pub items: Vec<SuggestionCard>,
}

/* ------------------------------ tickets ------------------------------ */

#[derive(Clone, Debug, PartialEq)]
pub struct TicketRowVm {
    pub key: TicketKey,
    pub priority: Badge,
    pub title: String,
    pub hotfix: bool,
    pub sub: String,
    pub jira: Badge,
    pub local: Option<Badge>,
    pub repos: Vec<RepoName>,
    pub flags: Vec<Badge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicketSection {
    pub heading: Option<String>,
    pub rows: Vec<TicketRowVm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicketListVm {
    pub group: Group,
    pub tabs: Vec<(Group, usize)>,
    pub sections: Vec<TicketSection>,
}

/* ------------------------------ ticket ------------------------------ */

#[derive(Clone, Debug, PartialEq)]
pub struct BannerVm {
    pub tone: Tone,
    pub title: String,
    pub lines: Vec<String>,
    pub actions: Vec<Btn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepperVm {
    pub steps: Vec<(String, StepState)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicketHeadVm {
    pub key: TicketKey,
    pub title: String,
    pub jira: Badge,
    pub local: Option<Badge>,
    pub hotfix: bool,
    pub uat_flag: Option<Badge>,
    pub actions: Vec<Btn>,
    pub banners: Vec<BannerVm>,
    pub stepper: StepperVm,
    /// `(tab, marker)`; the marker is a small dot on the Review tab when new commits arrived.
    pub tabs: Vec<(TicketTab, bool)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeployChip {
    pub text: String,
    pub tone: Tone,
    pub run: u32,
    pub step: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BranchCell {
    Chosen { name: Branch, manual: bool },
    Ambiguous(Vec<Btn>),
    Baseline(Branch),
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepoRowVm {
    pub repo: RepoName,
    pub branch: BranchCell,
    pub pr: Option<String>,
    pub deploy: Option<DeployChip>,
    pub touched: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommentVm {
    pub who: String,
    pub initials: String,
    pub at: String,
    pub body: Vec<Block>,
    pub is_new: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverviewVm {
    pub key: TicketKey,
    pub description: Vec<Block>,
    pub acceptance: Vec<String>,
    pub subtasks: Vec<(bool, String)>,
    pub repos: Vec<RepoRowVm>,
    pub comments: Vec<CommentVm>,
    pub attachments: Vec<(String, String)>,
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThreadVm {
    pub author: String,
    pub initials: String,
    pub mine: bool,
    pub resolved: bool,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffLineVm {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
    /// `n<new>` or `o<old>`: where an inline comment attaches.
    pub anchor: LineAnchor,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiffRow {
    Line {
        line: DiffLineVm,
        threads: Vec<ThreadVm>,
        composer: bool,
    },
    Pair {
        left: Option<DiffLineVm>,
        right: Option<DiffLineVm>,
        threads: Vec<ThreadVm>,
        composer: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct HunkVm {
    pub index: usize,
    pub of: usize,
    pub adds: usize,
    pub dels: usize,
    pub viewed: bool,
    pub toggle: Intent,
    pub rows: Vec<DiffRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileRowVm {
    pub path: String,
    pub adds: u32,
    pub dels: u32,
    pub progress: Option<String>,
    pub selected: bool,
    pub select: Intent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileGroupVm {
    pub label: String,
    pub files: Vec<FileRowVm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrHeadVm {
    pub title: String,
    pub source: String,
    pub dest: Badge,
    pub reviewers: Vec<Badge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReviewVm {
    pub key: TicketKey,
    /// The pull request being read (0 when there is none).
    pub pr_id: PrNumber,
    pub uat: Option<BannerVm>,
    pub stale: Option<BannerVm>,
    pub since_toggle: Option<(Btn, Btn, bool)>,
    pub prs: Vec<(String, bool, Intent)>,
    pub diff_mode: DiffMode,
    pub pr: Option<PrHeadVm>,
    pub groups: Vec<FileGroupVm>,
    pub file_path: String,
    pub hunks: Vec<HunkVm>,
    pub thread_count: usize,
    pub mark_reviewed: Btn,
    pub request_changes: Btn,
    pub approve: Btn,
    pub note: String,
    /// The inline comment being written: `(path, anchor)`.
    pub composer: Option<(String, LineAnchor)>,
    pub empty: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnvRow {
    pub repo: RepoName,
    pub role: Badge,
    pub branch: Branch,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TestVm {
    Inactive {
        why: String,
        errors: Vec<String>,
        plan: Vec<EnvRow>,
        overlay: Option<String>,
        activate: Option<Btn>,
    },
    Active {
        checklist: Vec<(String, bool)>,
        done: usize,
        notes: String,
        env: Vec<EnvRow>,
        overlay: Option<String>,
        actions: Vec<Btn>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PrepRow {
    Ready {
        repo: RepoName,
        commits: u32,
        files: u32,
        uat_before: String,
        merge: String,
        overlaps: Vec<String>,
    },
    AlreadyPushed {
        repo: RepoName,
        commit: String,
    },
    Conflict {
        repo: RepoName,
        with: TicketKey,
        files: String,
        comment: Option<Btn>,
        sent: Option<String>,
    },
    Blocked {
        repo: RepoName,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrePushGroup {
    pub repo: RepoName,
    pub items: Vec<(String, bool, Intent)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IntegrateBody {
    Active {
        prep: Option<Vec<PrepRow>>,
        prepush: Vec<PrePushGroup>,
        prepare: Btn,
        push: Btn,
    },
    Pushed {
        rows: Vec<(RepoName, String, String)>,
        new_commits: bool,
    },
    Inactive(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunRow {
    pub repo: RepoName,
    pub chip: DeployChip,
    pub rerun: Option<Btn>,
    pub failure: Option<(String, u32, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnnounceBody {
    Unavailable,
    Compose(Btn),
    Draft {
        id: DraftId,
        text: String,
        post: Btn,
    },
    Posted {
        text: String,
        moved: Option<Badge>,
        transition: Option<Btn>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AfterRow {
    pub label: String,
    pub badge: Badge,
    pub approve: Option<Btn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShipVm {
    pub key: TicketKey,
    pub integrate_state: StepState,
    pub integrate: IntegrateBody,
    pub runs_state: StepState,
    pub runs: Vec<RunRow>,
    pub uat_moved: Option<String>,
    pub announce_state: StepState,
    pub announce: AnnounceBody,
    pub after_state: StepState,
    pub after_text: String,
    pub approve_all: Option<Btn>,
    pub after: Vec<AfterRow>,
}

/* ------------------------------ environment ------------------------------ */

#[derive(Clone, Debug, PartialEq)]
pub struct UatItem {
    pub key: TicketKey,
    pub title: String,
    pub commit: String,
    pub chip: Option<DeployChip>,
    pub jira: Badge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UatColumn {
    pub repo: RepoName,
    pub host: String,
    pub items: Vec<UatItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OnUatVm {
    pub columns: Vec<UatColumn>,
    pub overlaps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WsRow {
    pub repo: RepoName,
    pub services: u32,
    pub branch: Branch,
    pub badges: Vec<Badge>,
    pub break_lock: Option<Btn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceVm {
    pub name: String,
    pub up: bool,
    pub order: String,
    pub rows: Vec<WsRow>,
    pub note: Option<String>,
    pub toggle: Btn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderVm {
    pub name: String,
    pub version: String,
    pub ready: bool,
    pub login_hint: String,
    pub toggle: Btn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsVm {
    /// Theme choices `(label, selected, intent)`; filled in by the session, which owns the choice.
    pub appearance: Vec<(String, bool, Intent)>,
    pub providers: Vec<ProviderVm>,
    pub wait_options: Vec<(String, bool, Btn)>,
    pub mapping: Vec<(String, String)>,
    pub repos: Vec<[String; 4]>,
    pub data: Vec<(String, String)>,
    pub reset: Btn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SimButton {
    pub label: String,
    pub on: bool,
    pub intent: Intent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SimGroup {
    pub title: String,
    pub buttons: Vec<SimButton>,
}

/* ------------------------------ sheets ------------------------------ */

/// The exact preview of a guarded command, as the gateway will show it.
///
/// A preview is the only way to get a [`Confirmed`]: [`Preview::confirm`] checks that the writer's tool is
/// available and that the ticket key was typed when the risk requires it. The store executes only a `Confirmed`.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    command: Command,
    pub title: String,
    pub ticket: Option<TicketKey>,
    pub risk: Risk,
    pub summary: String,
    /// Literal payload: refspecs, comment body, transition target.
    pub payload: Vec<String>,
    pub facts: Vec<String>,
    /// The text that must be typed to confirm (the ticket key for the uat push).
    pub type_key: Option<String>,
    pub confirm_label: String,
    /// Set when the writer's tool is missing or signed out: the sheet shows this instead.
    pub blocked: Option<String>,
}

impl Preview {
    pub fn new(
        command: Command,
        title: impl Into<String>,
        risk: Risk,
        summary: impl Into<String>,
        confirm_label: impl Into<String>,
    ) -> Self {
        Self {
            command,
            title: title.into(),
            ticket: None,
            risk,
            summary: summary.into(),
            payload: Vec::new(),
            facts: Vec::new(),
            type_key: None,
            confirm_label: confirm_label.into(),
            blocked: None,
        }
    }

    pub fn command(&self) -> &Command {
        &self.command
    }

    /// Whether `typed` satisfies the preview's requirements right now.
    pub fn can_confirm(&self, typed: &str) -> bool {
        self.blocked.is_none() && self.type_key.as_ref().is_none_or(|k| typed.trim() == k)
    }

    /// The only constructor of [`Confirmed`].
    pub fn confirm(self, typed: &str) -> Result<Confirmed, Unconfirmed> {
        if let Some(why) = self.blocked {
            return Err(Unconfirmed::Blocked(why));
        }
        if self.type_key.as_ref().is_some_and(|k| typed.trim() != k) {
            return Err(Unconfirmed::WrongKey);
        }
        Ok(Confirmed(self.command))
    }
}

/// A command the user has confirmed against its preview. Only [`Preview::confirm`] makes one.
#[derive(Debug)]
pub struct Confirmed(Command);

impl Confirmed {
    pub fn command(&self) -> &Command {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unconfirmed {
    Blocked(String),
    WrongKey,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReportRow {
    pub repo: RepoName,
    pub detail: String,
    pub ok: bool,
}

/// Result of a local operation (activation, park), shown as a progress list.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportVm {
    pub title: String,
    pub rows: Vec<ReportRow>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BaselineVm {
    pub key: TicketKey,
    pub choices: Vec<(super::intent::Baseline, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Warn,
    Bad,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    pub kind: ToastKind,
    pub undo: Option<Intent>,
    /// Seconds left before it disappears.
    pub ttl: f32,
}

/// Why a claim was refused: one ticket in hand at a time.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaimBlock {
    pub blocker: TicketKey,
    pub title: String,
    pub what: String,
    pub active: bool,
}

/// A repo that is locked: by another operation, or by a leftover `index.lock`.
#[derive(Clone, Debug, PartialEq)]
pub struct Busy {
    pub repo: RepoName,
    pub stale: bool,
    pub message: String,
}

//! One view-model type per screen area. The store builds them; the views draw them.

use super::common::*;
use super::ids::*;
use super::intent::{Command, DiffMode, Intent};
use super::services::ServiceIndicatorVm;

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
    /// The footer's services dot; `None` when there is no open workspace with services.
    pub services: Option<ServiceIndicatorVm>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RightRow {
    Kv(Kv),
    Text(String),
    Muted(String),
    /// A wrapped run of tags (components, labels).
    Tags(Vec<Badge>),
    Item {
        head: Vec<Badge>,
        key: Option<TicketKey>,
        title: String,
        sub: Option<String>,
    },
    /// One line about a ticket that opens it when clicked: key and title, with a mark for what is urgent.
    Ticket {
        key: TicketKey,
        title: String,
        /// A hotfix or a high priority (`text` says which); drawn as a mark before the key.
        mark: Option<Badge>,
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
    /// The ticket's priority when it is High or Highest: drawn as a mark beside the key, as in the ticket table.
    pub priority: Option<Badge>,
    /// Where clicking the row goes (the ticket, at the tab that matters). `None`: the row is not clickable.
    pub open: Option<Intent>,
    pub title: String,
    /// What the row's first line says instead of `title`: a claim row is about the ticket, so it carries the
    /// ticket's own title.
    pub headline: Option<String>,
    pub reason: String,
    /// What the second line says instead of `reason` (a claim row: the ticket's type and assignee).
    pub meta: Option<String>,
    /// When Jira last changed the ticket: `("2d ago", "Updated 2026-09-15 05:17")`, for a claim row.
    pub updated: Option<(String, String)>,
    /// Short facts shown in place of the reason when there are any.
    pub chips: Vec<Badge>,
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
    pub empty: EmptyVm,
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
    /// When Jira last changed the ticket: `("2d ago", "Updated 2026-09-15 05:17")`; `None` when unknown.
    pub updated: Option<(String, String)>,
    pub local: Option<Badge>,
    pub repos: Vec<RepoName>,
    pub flags: Vec<Badge>,
    /// Priority is shown only when it is an exception (High, Highest).
    pub urgent: bool,
    pub new_comments: bool,
}

/// One thing the ticket table can be narrowed by.
#[derive(Clone, Debug, PartialEq)]
pub enum TicketFilter {
    Repo(RepoName),
    Jira(String),
    Priority(String),
    Hotfix,
    NewComments,
}

/// What is narrowing the table. Rows must pass every kind that has something chosen (any one of its choices).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TicketFilters {
    pub repos: Vec<RepoName>,
    pub jira: Vec<String>,
    pub priority: Vec<String>,
    pub hotfix: bool,
    pub new_comments: bool,
}

impl TicketFilters {
    pub fn is_on(&self, f: &TicketFilter) -> bool {
        match f {
            TicketFilter::Repo(r) => self.repos.contains(r),
            TicketFilter::Jira(j) => self.jira.contains(j),
            TicketFilter::Priority(p) => self.priority.contains(p),
            TicketFilter::Hotfix => self.hotfix,
            TicketFilter::NewComments => self.new_comments,
        }
    }

    pub fn toggle(&mut self, f: TicketFilter) {
        fn flip<T: PartialEq>(v: &mut Vec<T>, x: T) {
            match v.iter().position(|y| *y == x) {
                Some(i) => {
                    v.remove(i);
                }
                None => v.push(x),
            }
        }
        match f {
            TicketFilter::Repo(r) => flip(&mut self.repos, r),
            TicketFilter::Jira(j) => flip(&mut self.jira, j),
            TicketFilter::Priority(p) => flip(&mut self.priority, p),
            TicketFilter::Hotfix => self.hotfix = !self.hotfix,
            TicketFilter::NewComments => self.new_comments = !self.new_comments,
        }
    }

    pub fn keeps(&self, r: &TicketRowVm) -> bool {
        (self.repos.is_empty() || r.repos.iter().any(|x| self.repos.contains(x)))
            && (self.jira.is_empty() || self.jira.contains(&r.jira.text))
            && (self.priority.is_empty() || self.priority.contains(&r.priority.text))
            && (!self.hotfix || r.hotfix)
            && (!self.new_comments || r.new_comments)
    }
}

/// Which column the table is sorted by, and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketSort {
    Key,
    Title,
    Jira,
}

/// The choices the filter menu offers (from the unfiltered list, so a chosen one never vanishes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TicketFilterOptions {
    pub repos: Vec<RepoName>,
    pub jira: Vec<String>,
    pub priority: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicketSection {
    pub heading: Option<String>,
    pub rows: Vec<TicketRowVm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicketListVm {
    /// The local state column is shown only where it is not implied (the list of all tickets).
    pub show_local: bool,
    /// What the empty list says when there is nothing at all (not when filters hide everything).
    /// Shown when there are no rows (the session swaps in a "nothing matches" one when filters hide them all).
    pub empty: EmptyVm,
    pub sections: Vec<TicketSection>,
    /// Filled in by the session, which owns the search text, filters and sort.
    pub options: TicketFilterOptions,
    pub filters: TicketFilters,
    pub sort: Option<(TicketSort, bool)>,
    /// Rows in this group before filtering, to tell "empty" from "nothing matches".
    pub total: usize,
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
    /// Every hunk of the file is marked viewed.
    pub viewed_all: bool,
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
    /// `(name, approved)`.
    pub reviewers: Vec<(String, bool)>,
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

/// What is on uat: the same table as every ticket list (deploy state is a flag like anywhere else), plus overlaps.
#[derive(Clone, Debug, PartialEq)]
pub struct OnUatVm {
    pub table: TicketListVm,
    pub overlaps: Vec<String>,
}

/// What one project's Services cell says: the count when things are fine, `1 of 3` when they
/// are not, and nothing claimed when the check could not run.
#[derive(Clone, Debug, PartialEq)]
pub enum ServicesVm {
    /// Every declared service is running — just the number.
    All(u32),
    /// Not all are running: shown as `1 of 3` in the warning colour.
    Partial { running: u32, declared: u32 },
    /// Docker could not be asked, so nothing is claimed: shown as `—`.
    Unknown,
}

impl ServicesVm {
    /// From what a health check found for this project; `running: None` is a check that could
    /// not run (Docker missing or its daemon down), not a project with nothing running.
    pub fn of(running: Option<u32>, declared: u32) -> Self {
        match running {
            None => ServicesVm::Unknown,
            Some(running) if running >= declared => ServicesVm::All(declared),
            Some(running) => ServicesVm::Partial {
                running,
                declared,
            },
        }
    }

    /// The count as the cell shows it.
    pub fn label(&self) -> String {
        match self {
            // Nothing to run says nothing: a bare `0` is the normal state for a project with no
            // services, not something to report.
            ServicesVm::All(0) => String::new(),
            ServicesVm::All(declared) => declared.to_string(),
            ServicesVm::Partial { running, declared } => format!("{running} of {declared}"),
            ServicesVm::Unknown => "—".to_string(),
        }
    }

    /// The warning colour only for something that is off; the default and the unknown stay plain.
    pub fn tone(&self) -> Option<Tone> {
        match self {
            ServicesVm::Partial { .. } => Some(Tone::Warn),
            ServicesVm::All(_) | ServicesVm::Unknown => None,
        }
    }
}

/// How the workspace's services stand when they are not all running. The view model carries
/// `None` instead of a value when everything runs (or there is nothing to run): the normal
/// state is silence, and only an exception gets a badge.
#[derive(Clone, Debug, PartialEq)]
pub enum HealthVm {
    /// Some or all of the declared services are stopped: `2 of 6 services`, `services down`.
    Stopped { running: u32, declared: u32 },
    /// Services are running but unhealthy: the names say which.
    Unhealthy { named: Vec<String> },
    /// Docker itself could not be asked; the reason says what to do about it.
    Unavailable { reason: String },
}

impl HealthVm {
    /// What a check that found the services not all running shows; `None` when they all run
    /// (or the workspace declares none): silence, the normal state.
    pub fn stopped(running: u32, declared: u32) -> Option<Self> {
        (declared > 0 && running < declared).then_some(HealthVm::Stopped { running, declared })
    }

    /// Services that are up but not healthy (`project/service` names), the loudest of the three.
    pub fn unhealthy(named: Vec<String>) -> Self {
        HealthVm::Unhealthy { named }
    }

    /// The state of a check that could not run at all, in the tone of a real blocker.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        HealthVm::Unavailable {
            reason: reason.into(),
        }
    }

    /// The one badge the strip shows, only ever for something that is off.
    pub fn badge(&self) -> Badge {
        match self {
            HealthVm::Stopped { running: 0, .. } => Badge::new("services down", Tone::Warn),
            HealthVm::Stopped { running, declared } => {
                Badge::new(format!("{running} of {declared} services"), Tone::Warn)
            }
            HealthVm::Unhealthy { named } => {
                Badge::new(format!("{} unhealthy", named.len()), Tone::Bad)
            }
            HealthVm::Unavailable { .. } => Badge::new("Docker unavailable", Tone::Bad),
        }
    }

    /// The quiet line under the strip: only a failed check has anything to say about what to
    /// do next.
    pub fn notice(&self) -> Option<String> {
        match self {
            HealthVm::Unavailable { reason } => Some(reason.clone()),
            HealthVm::Unhealthy { named } => {
                let shown = named.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
                let more = if named.len() > 2 {
                    format!(" and {} more", named.len() - 2)
                } else {
                    String::new()
                };
                Some(format!("Unhealthy: {shown}{more}"))
            }
            HealthVm::Stopped { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WsRow {
    pub repo: RepoName,
    pub services: ServicesVm,
    pub branch: Branch,
    pub badges: Vec<Badge>,
    pub break_lock: Option<Btn>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceVm {
    pub name: String,
    pub up: bool,
    /// What to say about the services when they are not all running; `None` is silence (all
    /// running, or none declared), the normal state.
    pub health: Option<HealthVm>,
    pub rows: Vec<WsRow>,
    pub note: Option<String>,
    /// Ask the engine again how the services stand.
    pub refresh: Btn,
    pub toggle: Btn,
    /// Move every project of the workspace to one branch; opens the picker that says what that
    /// would do first.
    pub switch_branch: Btn,
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
    /// How often the app syncs by itself: `(label, selected, button)`; the first is "Off".
    pub sync_options: Vec<(String, bool, Btn)>,
    pub mapping: Vec<MappingRow>,
    /// The mapping fields whose text differs from what is stored; filled in by the session, which owns the text.
    pub edited: Vec<MappingKey>,
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

/// The sync log screen: the runs on disk and the text of the selected one.
#[derive(Clone, Debug, PartialEq)]
pub struct LogsVm {
    /// Newest first.
    pub runs: Vec<LogRunVm>,
    /// Says how many are kept and where that is set.
    pub note: String,
    pub selected: Option<String>,
    /// The selected log, one entry per line. Shared so a frame does not copy a large file.
    pub lines: std::rc::Rc<Vec<String>>,
    /// What the screen says when there are no runs: what this place is for, and what to do next.
    /// A run belongs to the workspace it was made in, so this also carries the reason when the
    /// open workspace has none yet.
    pub empty: EmptyVm,
}

/* ------------------------------ workspaces ------------------------------ */

/// One saved workspace, as the picker and the workspace list show it.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceItemVm {
    pub name: WorkspaceName,
    /// Its projects (repos), in the order they start. A workspace can span several folders, so these and not a
    /// directory say what it is.
    pub projects: Vec<String>,
    pub repos: u32,
    /// Its services are running.
    pub up: bool,
    /// The one this window has open.
    pub current: bool,
    /// `3h ago`, or `never` for one that has not been opened.
    pub last_used: String,
}

/// The saved workspaces, most recently used first, and which one (if any) is selected.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspacesVm {
    pub current: Option<WorkspaceName>,
    pub items: Vec<WorkspaceItemVm>,
}

/// What is shown when no workspace is selected (and from the workspace list): the state of the tools, the saved
/// workspaces and how to make one.
#[derive(Clone, Debug, PartialEq)]
pub struct WelcomeVm {
    pub providers: Vec<ProviderVm>,
    pub workspaces: Vec<WorkspaceItemVm>,
    /// The command that creates a workspace.
    pub init_command: String,
    pub current: Option<WorkspaceName>,
}

/// The workspace menu of the title bar.
#[derive(Clone, Debug, PartialEq)]
pub struct PickerVm {
    /// What is typed in the search field.
    pub query: String,
    /// The open workspace, when it matches the search.
    pub current: Option<WorkspaceItemVm>,
    /// The others, most recently used first, narrowed by the search.
    pub recent: Vec<WorkspaceItemVm>,
    /// How many saved workspaces there are before the search narrows them.
    pub total: usize,
    pub init_command: String,
}

/* ------------------------------ opening and closing ------------------------------ */

/// Where one step of a sequence is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    Waiting,
    Running,
    Done,
    /// It ran and went wrong; what to do is in the detail.
    Failed,
    /// It was not run because something before it made it pointless; the detail says what.
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepVm {
    pub label: String,
    /// The result once there is one (`develop, clean`, `3 services`, why it failed); empty while waiting.
    pub detail: String,
    pub state: ProgressState,
}

/// One half of a sequence: a switch is `Closing a` then `Opening b`.
#[derive(Clone, Debug, PartialEq)]
pub struct PhaseVm {
    pub title: String,
    pub steps: Vec<StepVm>,
}

/// How a sequence stands. Only `Running` keeps the modal from being left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceState {
    Running,
    /// Everything went fine; the modal closes by itself after a short beat.
    Ready,
    /// Finished with something failed: the person decides.
    NeedsDecision,
}

/// The opening/closing modal: what is being done to which workspace and how far it has got.
#[derive(Clone, Debug, PartialEq)]
pub struct SequenceVm {
    pub title: String,
    pub phases: Vec<PhaseVm>,
    pub state: SequenceState,
    /// `4 of 9`.
    pub progress: String,
    /// What the footer offers once the sequence is not running; empty while it runs.
    pub actions: Vec<Btn>,
}

/// The modal listing every saved workspace, with a search.
#[derive(Clone, Debug, PartialEq)]
pub struct AllWorkspacesVm {
    pub query: String,
    pub items: Vec<WorkspaceItemVm>,
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are silence: nothing runs, nothing needs a badge.
    #[test]
    fn all_services_running_says_nothing() {
        assert_eq!(HealthVm::stopped(6, 6), None);
        // A workspace with no services to run is not "down".
        assert_eq!(HealthVm::stopped(0, 0), None);
        assert_eq!(ServicesVm::of(Some(3), 3), ServicesVm::All(3));
        assert_eq!(ServicesVm::of(Some(0), 0), ServicesVm::All(0));
        assert_eq!(ServicesVm::of(Some(3), 3).label(), "3");
        assert_eq!(ServicesVm::of(Some(3), 3).tone(), None);
        // A project with nothing to run says nothing at all: a bare `0` is the normal state.
        assert_eq!(ServicesVm::of(Some(0), 0).label(), "");
        assert_eq!(ServicesVm::of(Some(0), 0).tone(), None);
    }

    /// What is off is named: `2 of 6 services` at the workspace, `1 of 3` in the row.
    #[test]
    fn stopped_services_say_how_many_are_missing() {
        assert_eq!(
            HealthVm::stopped(4, 6),
            Some(HealthVm::Stopped {
                running: 4,
                declared: 6
            })
        );
        assert_eq!(
            HealthVm::stopped(0, 6).unwrap().badge(),
            Badge::new("services down", Tone::Warn)
        );
        assert_eq!(
            HealthVm::stopped(4, 6).unwrap().badge(),
            Badge::new("4 of 6 services", Tone::Warn)
        );
        // A stopped workspace has nothing to say under the strip; the rows say which.
        assert_eq!(HealthVm::stopped(0, 6).unwrap().notice(), None);

        let row = ServicesVm::of(Some(1), 3);
        assert_eq!(
            row,
            ServicesVm::Partial {
                running: 1,
                declared: 3
            }
        );
        assert_eq!(row.label(), "1 of 3");
        assert_eq!(row.tone(), Some(Tone::Warn));
    }

    /// A check that could not run claims nothing: the reason (with what to do) is the report.
    #[test]
    fn a_failed_check_claims_nothing_and_says_why() {
        let health = HealthVm::unavailable("Docker is not running. Start Docker Desktop, then press Refresh.");
        assert_eq!(
            health.badge(),
            Badge::new("Docker unavailable", Tone::Bad)
        );
        assert_eq!(
            health.notice(),
            Some("Docker is not running. Start Docker Desktop, then press Refresh.".to_string())
        );
        assert_eq!(ServicesVm::of(None, 3), ServicesVm::Unknown);
        assert_eq!(ServicesVm::of(None, 3).label(), "—");
        assert_eq!(ServicesVm::of(None, 3).tone(), None);
    }

    /// A service that runs but is not healthy is the loudest badge, and names the culprits.
    #[test]
    fn unhealthy_services_are_named() {
        let health = HealthVm::unhealthy(vec!["api/db".into(), "web/api".into(), "worker/queue".into()]);
        assert_eq!(health.badge(), Badge::new("3 unhealthy", Tone::Bad));
        assert_eq!(
            health.notice(),
            Some("Unhealthy: api/db, web/api and 1 more".to_string())
        );
    }
}

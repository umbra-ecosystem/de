//! The seam between the views and whatever supplies the data.
//!
//! The showcase implements [`Store`] with an in-memory simulation ([`crate::sim::Sim`]). The `de-app` crate will
//! implement it over `de-core` and `de next --json`. The views and [`crate::session::Session`] only know this trait.
//!
//! Writes are split by type. A [`LocalCommand`] is dispatched directly. A [`RemoteCommand`] can only be previewed;
//! it runs only as a [`Confirmed`], which only [`Preview::confirm`] produces. There is no other way in.

use std::collections::BTreeSet;

use crate::vm::*;

/// Which diff the review screen is showing; UI state the session owns and passes down.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewSel {
    pub pr: Option<PrNumber>,
    pub file: usize,
    pub mode: DiffMode,
    pub since: SinceMode,
    pub viewed: BTreeSet<HunkId>,
    /// `(pr, file path, anchor)` of the open inline composer.
    pub composer: Option<(PrNumber, String, LineAnchor)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToastSpec {
    pub text: String,
    pub kind: ToastKind,
    pub undo: Option<Undo>,
}

/// What a command did. Exactly one of these; there is no "ok with errors".
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done {
        toast: Option<ToastSpec>,
        /// Shown as a progress list (activation, park).
        report: Option<ReportVm>,
    },
    /// Nothing was changed; these are the reasons.
    Refused(Vec<String>),
    /// A hotfix needs a baseline choice before it can be activated.
    NeedsBaseline,
    /// Refused because another ticket is in hand.
    Blocked(ClaimBlock),
    /// Refused because a repo is locked.
    Busy(Busy),
}

impl Outcome {
    pub fn ok() -> Self {
        Outcome::Done {
            toast: None,
            report: None,
        }
    }

    pub fn fail(error: impl Into<String>) -> Self {
        Outcome::Refused(vec![error.into()])
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Outcome::Done { .. })
    }

    /// Attach a toast to a successful outcome; other outcomes are returned unchanged.
    pub fn with_toast(self, text: impl Into<String>, kind: ToastKind, undo: Option<Undo>) -> Self {
        match self {
            Outcome::Done { report, .. } => Outcome::Done {
                toast: Some(ToastSpec {
                    text: text.into(),
                    kind,
                    undo,
                }),
                report,
            },
            other => other,
        }
    }

    pub fn with_report(self, report: ReportVm) -> Self {
        match self {
            Outcome::Done { toast, .. } => Outcome::Done {
                toast,
                report: Some(report),
            },
            other => other,
        }
    }
}

pub trait Store {
    /* ---- shell ---- */
    fn counts(&self) -> Counts;
    fn status(&self) -> StatusVm;
    fn tab_info(&self, key: &TicketKey) -> Option<TabInfo>;
    fn right_panel(&self, route: &Route) -> Vec<RightSection>;
    fn palette(&self, query: &str) -> Vec<PaletteItem>;

    /* ---- screens ---- */
    fn next(&self, show_all: bool) -> NextVm;
    fn attention(&self) -> AttentionVm;
    fn tickets(&self, group: Group) -> TicketListVm;
    fn ticket_exists(&self, key: &TicketKey) -> bool;
    fn ticket_head(&self, key: &TicketKey, tab: TicketTab) -> Option<TicketHeadVm>;
    /// `seen` is the number of comments already seen when the tab was opened.
    fn overview(&self, key: &TicketKey, seen: Option<u32>) -> Option<OverviewVm>;
    fn review(&self, key: &TicketKey, sel: &ReviewSel) -> Option<ReviewVm>;
    fn test(&self, key: &TicketKey) -> Option<TestVm>;
    fn ship(&self, key: &TicketKey) -> Option<ShipVm>;
    fn timeline(&self, key: &TicketKey) -> Vec<AuditRow>;
    fn on_uat(&self) -> OnUatVm;
    fn workspace(&self) -> WorkspaceVm;
    /// The saved workspaces, most recently used first, and the open one.
    fn workspaces(&self) -> WorkspacesVm;
    /// What is shown with no workspace open: tool state, the saved workspaces, how to make one.
    fn welcome(&self) -> WelcomeVm;
    /// The opening or closing sequence under way (or waiting for a decision), if any.
    fn sequence(&self) -> Option<SequenceVm>;
    /// Every saved workspace matching `query`, for the modal.
    fn all_workspaces(&self, query: &str) -> AllWorkspacesVm;
    /// Every service of the open workspace and how each one stands, for the modal.
    fn services_modal(&self) -> ServicesModalVm;
    /// The branches of the open workspace for the modal that picks one, narrowed by `query`; `chosen`
    /// is what has been picked in it, so the modal can say what switching would do. `reading` while
    /// the first check of the workspace is still running: nothing is claimed before it is known.
    fn branch_picker(&self, query: &str, chosen: Option<&Branch>) -> BranchPickerVm;
    fn audit(&self) -> Vec<AuditRow>;
    fn settings(&self) -> SettingsVm;
    /// The sync logs on disk, newest first, and the line that says how many are kept.
    fn logs(&self) -> (Vec<LogRunVm>, String);
    /// The text of one log. Total: a file that is gone reads as a sentence, not an error.
    fn log_text(&self, id: &str) -> String;
    fn simulate(&self) -> Vec<SimGroup>;
    /// The comment count at which a ticket was last marked seen (for "new" markers).
    fn comments_seen(&self, key: &TicketKey) -> u32;
    /// The raw output `de providers probe` would print (fake here).
    fn diagnose(&self) -> String;

    /* ---- sheets ---- */
    fn baseline_choices(&self, key: &TicketKey) -> BaselineVm;
    /// The preview of a remote write. Total by type: every remote command has one. `Err` only when its target is gone.
    fn preview(&self, command: &RemoteCommand) -> Result<Preview, String>;
    /// A local command that still asks first (park, remove a lock, stop the workspace).
    fn local_guard(&self, command: &LocalCommand) -> Option<Preview>;

    /* ---- effects ---- */
    fn dispatch(&mut self, command: LocalCommand) -> Outcome;
    /// Run a command the user confirmed against its preview. The only path for a remote write.
    fn execute(&mut self, confirmed: Confirmed) -> Outcome;
    /// Advance simulated time and deliver background events; returns toasts raised meanwhile.
    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)>;
}

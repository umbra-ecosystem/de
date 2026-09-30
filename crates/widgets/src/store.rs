//! The seam between the views and whatever supplies the data.
//!
//! The showcase implements [`Store`] with an in-memory simulation ([`crate::sim::Sim`]). The `de-app` crate will
//! implement it over `de-core` and `de next --json`. The views and [`crate::session::Session`] only know this trait.

use crate::vm::*;

/// Which diff the review screen is showing; UI state the session owns and passes down.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewSel {
    pub pr: Option<u32>,
    pub file: usize,
    pub mode: DiffMode,
    pub since: SinceMode,
    pub viewed: Vec<String>,
    /// `(pr, file path, anchor)` of the open inline composer.
    pub composer: Option<(u32, String, String)>,
}

/// The key under which a hunk's "viewed" state is remembered.
pub fn hunk_key(ticket: &str, pr: u32, since: bool, path: &str, hunk: usize) -> String {
    format!(
        "{ticket}:{pr}{}:{path}:{hunk}",
        if since { "s" } else { "" }
    )
}

/// What a command did. The session turns it into toasts and sheets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub errors: Vec<String>,
    pub toast: Option<(String, ToastKind, Option<Undo>)>,
    /// A hotfix needs a baseline choice before it can be activated.
    pub need_baseline: bool,
    pub report: Option<ReportVm>,
    /// Refused because another ticket is in hand.
    pub blocker: Option<ClaimBlock>,
    /// Refused because a repo is locked.
    pub busy: Option<Busy>,
}

impl Outcome {
    pub fn ok() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    pub fn fail(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            errors: vec![error.into()],
            ..Self::default()
        }
    }

    pub fn blocked(block: ClaimBlock) -> Self {
        Self {
            blocker: Some(block),
            ..Self::default()
        }
    }

    pub fn busy(busy: Busy) -> Self {
        Self {
            busy: Some(busy),
            ..Self::default()
        }
    }

    pub fn with_toast(
        mut self,
        text: impl Into<String>,
        kind: ToastKind,
        undo: Option<Undo>,
    ) -> Self {
        self.toast = Some((text.into(), kind, undo));
        self
    }
}

pub trait Store {
    /* ---- shell ---- */
    fn counts(&self) -> Counts;
    fn status(&self) -> StatusVm;
    fn tab_info(&self, key: &str) -> Option<TabInfo>;
    fn right_panel(&self, route: &Route) -> Vec<RightSection>;
    fn palette(&self, query: &str) -> Vec<PaletteItem>;

    /* ---- screens ---- */
    fn next(&self, show_all: bool) -> NextVm;
    fn attention(&self) -> AttentionVm;
    fn tickets(&self, group: Group) -> TicketListVm;
    fn ticket_exists(&self, key: &str) -> bool;
    fn ticket_title(&self, key: &str) -> String;
    fn ticket_head(&self, key: &str, tab: TicketTab) -> Option<TicketHeadVm>;
    /// `seen` is the number of comments already seen when the tab was opened.
    fn overview(&self, key: &str, seen: Option<u32>) -> Option<OverviewVm>;
    fn review(&self, key: &str, sel: &ReviewSel) -> Option<ReviewVm>;
    fn test(&self, key: &str) -> Option<TestVm>;
    fn ship(&self, key: &str) -> Option<ShipVm>;
    fn timeline(&self, key: &str) -> Vec<AuditRow>;
    fn on_uat(&self) -> OnUatVm;
    fn workspace(&self) -> WorkspaceVm;
    fn audit(&self) -> Vec<AuditRow>;
    fn settings(&self) -> SettingsVm;
    fn simulate(&self) -> Vec<SimGroup>;
    /// The comment count at which a ticket was last marked seen (for "new" markers).
    fn comments_seen(&self, key: &str) -> u32;
    /// The raw output `de providers probe` would print (fake here).
    fn diagnose(&self) -> String;

    /* ---- sheets ---- */
    fn baseline_choices(&self, key: &str) -> BaselineVm;
    /// `Some` for every command that writes to a remote: the session shows it before calling `dispatch`.
    fn preview(&self, command: &Command) -> Option<Preview>;

    /* ---- effects ---- */
    fn dispatch(&mut self, command: &Command) -> Outcome;
    /// Advance simulated time and deliver background events; returns toasts raised meanwhile.
    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)>;
}

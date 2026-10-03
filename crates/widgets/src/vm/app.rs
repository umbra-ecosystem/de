//! The whole window as one view model: what the shell, the screen, the sheet and the toasts show right now.

use super::branchpick::BranchPickerVm;
use super::common::*;
use super::ids::*;
use super::intent::{Command, ComposeKind, Field, Then};
use super::screens::*;
use super::services::ServicesModalVm;

#[derive(Clone, Debug, PartialEq)]
pub struct ShellTab {
    pub route: Route,
    pub key: Option<TicketKey>,
    pub label: String,
    pub title: String,
    pub selected: bool,
    pub pinned: bool,
    pub bad: bool,
    pub unseen: bool,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavItem {
    pub label: String,
    pub route: Route,
    pub count: usize,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavSection {
    pub title: String,
    pub items: Vec<NavItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TicketBody {
    Overview(OverviewVm),
    Review(ReviewVm),
    Test(TestVm),
    Ship(ShipVm),
    Timeline(Vec<AuditRow>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScreenVm {
    Next(NextVm),
    Tickets(TicketListVm),
    Ticket {
        head: Box<TicketHeadVm>,
        tab: TicketTab,
        body: Box<TicketBody>,
        /// Text of the free-comment field (Overview).
        comment: String,
        checklist_add: String,
    },
    OnUat(OnUatVm),
    Workspace(WorkspaceVm),
    Audit(Vec<AuditRow>),
    Logs(LogsVm),
    Welcome(WelcomeVm),
    Settings(SettingsVm),
    Missing(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum SheetVm {
    Confirm {
        preview: Preview,
        typed: String,
        can_confirm: bool,
    },
    Baseline(BaselineVm),
    Report(ReportVm),
    Compose {
        kind: ComposeKind,
        title: String,
        placeholder: String,
        confirm_label: String,
        text: String,
    },
    Palette {
        query: String,
        items: Vec<PaletteItem>,
    },
    ClaimBlocked {
        key: TicketKey,
        block: ClaimBlock,
        then: Then,
    },
    Busy {
        busy: Busy,
        retry: Command,
    },
    Diagnose(String),
    CloseTab(TicketKey),
    /// Every saved workspace, searchable: the landing page shows only a few.
    AllWorkspaces(AllWorkspacesVm),
    /// The branches of the open workspace, searchable: pick one to move the whole workspace to.
    Branches(BranchPickerVm),
    /// Every service of the open workspace and how each one stands.
    Services(ServicesModalVm),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppVm {
    pub route: Route,
    pub nav: Vec<NavSection>,
    pub tabs: Vec<ShellTab>,
    pub status: StatusVm,
    /// The title bar's workspace button.
    pub workspace: WorkspaceChip,
    /// The workspace menu, when open.
    pub picker: Option<PickerVm>,
    pub attention_count: usize,
    pub attention: Option<AttentionVm>,
    pub simulate: Option<Vec<SimGroup>>,
    pub right_title: String,
    pub right: Vec<RightSection>,
    pub right_open: bool,
    pub theme: ThemeChoice,
    pub screen: ScreenVm,
    pub sheet: Option<SheetVm>,
    /// Opening or closing a workspace is in progress or waiting for a decision: a modal nothing else can be done
    /// beside.
    pub sequence: Option<SequenceVm>,
    pub toasts: Vec<Toast>,
}

/// The title bar's workspace button: the open workspace's name, or a prompt to pick one.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceChip {
    pub label: String,
    /// No workspace is open.
    pub none: bool,
}

/// The text a field currently holds, for the views to put into their inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldText {
    pub field: Field,
    pub text: String,
}

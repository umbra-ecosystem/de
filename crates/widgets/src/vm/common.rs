//! Types shared by every view model. Plain data: no GPUI, no engine.

use super::ids::*;
use super::intent::Intent;

/// Colour intent. The views map it to theme colours; the view models never name a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Bad,
    Hot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    pub text: String,
    pub tone: Tone,
}

impl Badge {
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
        }
    }
}

/// What an empty screen says: what this place is for, and the next useful thing to do from here.
#[derive(Clone, Debug, PartialEq)]
pub struct EmptyVm {
    pub title: String,
    pub hint: String,
    pub actions: Vec<Btn>,
}

impl EmptyVm {
    pub fn new(title: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            hint: hint.into(),
            actions: Vec::new(),
        }
    }

    pub fn action(mut self, a: Btn) -> Self {
        self.actions.push(a);
        self
    }

    pub fn action_if(self, a: Option<Btn>) -> Self {
        match a {
            Some(a) => self.action(a),
            None => self,
        }
    }
}

/// `secs` seconds ago in the lists' words. Anything under a minute, or from the future (clock skew), is "just now".
pub fn ago(secs: i64) -> String {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    match secs {
        s if s < MIN => "just now".to_string(),
        s if s < HOUR => format!("{}m ago", s / MIN),
        s if s < DAY => format!("{}h ago", s / HOUR),
        s if s < 14 * DAY => format!("{}d ago", s / DAY),
        s if s < 60 * DAY => format!("{}w ago", s / (7 * DAY)),
        s if s < 365 * DAY => format!("{}mo ago", s / (30 * DAY)),
        s => format!("{}y ago", s / (365 * DAY)),
    }
}

/// A button the view model offers. The view renders it and emits `intent` when clicked.
#[derive(Clone, Debug, PartialEq)]
pub struct Btn {
    pub label: String,
    pub intent: Intent,
    pub primary: bool,
    pub enabled: bool,
    /// Why it is disabled (or extra detail), shown as a tooltip.
    pub hint: Option<String>,
}

impl Btn {
    pub fn new(label: impl Into<String>, intent: Intent) -> Self {
        Self {
            label: label.into(),
            intent,
            primary: false,
            enabled: true,
            hint: None,
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn primary_if(mut self, yes: bool) -> Self {
        self.primary = yes;
        self
    }

    pub fn disabled(mut self, why: Option<String>) -> Self {
        if why.is_some() {
            self.enabled = false;
            self.hint = why;
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Pool,
    Mine,
    Active,
    Parked,
    Awaiting,
    Returned,
    Done,
    All,
}

impl Group {
    pub const LIST: [Group; 8] = [
        Group::Pool,
        Group::Mine,
        Group::Active,
        Group::Parked,
        Group::Awaiting,
        Group::Returned,
        Group::Done,
        Group::All,
    ];
    /// The groups shown in the sidebar (everything but `All`).
    pub const NAV: [Group; 7] = [
        Group::Pool,
        Group::Mine,
        Group::Active,
        Group::Parked,
        Group::Awaiting,
        Group::Returned,
        Group::Done,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::Pool => "Review pool",
            Group::Mine => "In review",
            Group::Active => "Active",
            Group::Parked => "Parked",
            Group::Awaiting => "Awaiting alpha",
            Group::Returned => "Returned",
            Group::Done => "Done",
            Group::All => "All tickets",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TicketTab {
    Overview,
    Review,
    Test,
    Ship,
    Timeline,
}

impl TicketTab {
    pub const LIST: [TicketTab; 5] = [
        TicketTab::Overview,
        TicketTab::Review,
        TicketTab::Test,
        TicketTab::Ship,
        TicketTab::Timeline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TicketTab::Overview => "Overview",
            TicketTab::Review => "Review",
            TicketTab::Test => "Test",
            TicketTab::Ship => "Ship",
            TicketTab::Timeline => "Timeline",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Next,
    Tickets(Group),
    Ticket { key: TicketKey, tab: TicketTab },
    OnUat,
    Workspace,
    Audit,
    /// Raw logs of sync runs.
    Logs,
    Settings,
    /// The saved workspaces and the tools' state: what is shown when no workspace is selected, and the list to
    /// pick one from.
    Welcome,
}

impl Route {
    /// Whether the screen shows a workspace's own data (its tickets, repos, audit log). Those need a workspace to
    /// be selected; the others (settings, sync logs, the workspace list) do not.
    pub fn needs_workspace(&self) -> bool {
        !matches!(self, Route::Settings | Route::Logs | Route::Welcome)
    }
}

impl Route {
    pub fn ticket(key: impl Into<TicketKey>, tab: TicketTab) -> Self {
        Route::Ticket {
            key: key.into(),
            tab,
        }
    }
}

/// Rich text as Jira serves it, reduced to what the screens draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(String),
    Heading(String),
    List(Vec<String>),
    Code(String),
    /// A numbered list; the items carry no numbers of their own.
    Numbered(Vec<String>),
    Quote(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Todo,
    Now,
    Done,
    Bad,
}

/// A setting of the Jira mapping that can be edited on the settings page. An empty text means "not set": the
/// default applies, and the field shows it as its placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MappingKey {
    ReviewStatus,
    AlphaStatus,
    UatStatus,
    ReturnedStatus,
    /// Comma-separated.
    DoneStatuses,
    /// Comma-separated.
    SignedOffStatuses,
    ReviewJql,
    AccountId,
}

impl MappingKey {
    pub const LIST: [MappingKey; 8] = [
        MappingKey::ReviewStatus,
        MappingKey::AlphaStatus,
        MappingKey::UatStatus,
        MappingKey::ReturnedStatus,
        MappingKey::DoneStatuses,
        MappingKey::SignedOffStatuses,
        MappingKey::ReviewJql,
        MappingKey::AccountId,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MappingKey::ReviewStatus => "Review status",
            MappingKey::AlphaStatus => "Alpha testing status",
            MappingKey::UatStatus => "UAT status",
            MappingKey::ReturnedStatus => "Returned status",
            MappingKey::DoneStatuses => "Done statuses",
            MappingKey::SignedOffStatuses => "Signed-off statuses",
            MappingKey::ReviewJql => "Review query (JQL)",
            MappingKey::AccountId => "My account id",
        }
    }

    /// What applies while the field is empty.
    pub fn placeholder(self) -> &'static str {
        match self {
            MappingKey::ReviewStatus => "In Review",
            MappingKey::AlphaStatus => "Alpha Testing",
            MappingKey::UatStatus => "UAT",
            MappingKey::ReturnedStatus => "Returned",
            MappingKey::DoneStatuses => "Done",
            MappingKey::SignedOffStatuses => "same as the done statuses",
            MappingKey::ReviewJql => "status = \"In Review\" AND updated >= -30d",
            MappingKey::AccountId => "not set: mentions of you are not flagged",
        }
    }

    /// The default when it is a plain value the field can show as real text (so an unset field reads like a set
    /// one). `None` for a default that is an explanation rather than a value.
    pub fn literal_default(self) -> Option<&'static str> {
        match self {
            MappingKey::ReviewStatus
            | MappingKey::AlphaStatus
            | MappingKey::UatStatus
            | MappingKey::ReturnedStatus
            | MappingKey::DoneStatuses => Some(self.placeholder()),
            MappingKey::SignedOffStatuses | MappingKey::ReviewJql | MappingKey::AccountId => None,
        }
    }

    /// Whether the text is a comma-separated list.
    pub fn is_list(self) -> bool {
        matches!(self, MappingKey::DoneStatuses | MappingKey::SignedOffStatuses)
    }
}

/// One editable row: the value as stored (empty when unset).
#[derive(Clone, Debug, PartialEq)]
pub struct MappingRow {
    pub key: MappingKey,
    pub value: String,
}

/// One sync run's log file, as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct LogRunVm {
    /// The store's own key for the file; sent back in [`Intent::SelectLog`](super::Intent::SelectLog).
    pub id: String,
    /// When the run started (UTC).
    pub when: String,
    pub size: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuditRow {
    pub at: String,
    pub action: String,
    pub repo: String,
    pub ticket: Option<TicketKey>,
    pub outcome: Badge,
    pub details: String,
}

/// One row of a key/value list (right panel, settings).
#[derive(Clone, Debug, PartialEq)]
pub struct Kv {
    pub key: String,
    pub value: Vec<Badge>,
}

impl Kv {
    pub fn text(key: &str, value: impl Into<String>) -> Self {
        Self {
            key: key.to_string(),
            value: vec![Badge::new(value, Tone::Neutral)],
        }
    }

    pub fn badges(key: &str, value: Vec<Badge>) -> Self {
        Self {
            key: key.to_string(),
            value,
        }
    }
}

/// The part of a ticket's life in which it is in your hands but not active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Claimed,
    Reviewing,
    Parked,
}

/// Which built-in theme mode the window uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const LIST: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }
}

//! What a view can ask for. Views only ever emit an [`Intent`]; they never mutate anything.
//!
//! `Intent` is UI state (navigation, selections, sheets). `Command` is a request to the [`Store`](crate::Store):
//! the thing the engine will eventually do. A command the store previews is confirmed in a sheet first.

use super::common::{Group, Route, TicketTab};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Baseline {
    Production,
    Develop,
    Uat,
}

impl Baseline {
    pub fn label(self) -> &'static str {
        match self {
            Baseline::Production => "production branch",
            Baseline::Develop => "develop",
            Baseline::Uat => "uat",
        }
    }
}

/// What to do once the ticket in hand has been parked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    Claim,
    StartReview,
    Reclaim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitKind {
    PullRequest,
    Threads,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Jira,
    GitHub,
}

/// Things the simulate panel can make the outside world do.
#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    Offline(bool),
    Conflict(Option<String>),
    OverlayLeak(Option<String>),
    UatMoves(bool),
    FailDeploy(Option<String>),
    Mention(String),
    SignOff(String),
    Returned(String),
    BackToReview(String),
    ExternalLock(String),
    StaleLock(String),
    PrArrives(String),
    ResolveThreads(String),
    NewCommits(String),
    UatAfterPush(String),
    SkipMinutes(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    Claim(String),
    Reviewed {
        key: String,
        prev_seq: Option<u32>,
        prev_status: String,
    },
    Response(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Sync,
    Claim(String),
    StartReview(String),
    MarkReviewed(String),
    Activate {
        key: String,
        baseline: Option<Baseline>,
    },
    Park(String),
    /// Park the ticket in hand (`from`), then continue with `key`.
    ParkAndContinue {
        from: String,
        key: String,
        then: Then,
    },
    Recheck,
    ToggleChecklist {
        key: String,
        index: usize,
    },
    AddChecklist {
        key: String,
        text: String,
    },
    SetNotes {
        key: String,
        text: String,
    },
    Choose {
        key: String,
        repo: String,
        branch: String,
    },
    Prepare(String),
    TogglePrePush {
        key: String,
        repo: String,
        index: usize,
    },
    PushUat(String),
    Rerun {
        key: String,
        repo: String,
    },
    ComposeDraft(String),
    EditDraft {
        key: String,
        id: String,
        text: String,
    },
    PostAndMove {
        key: String,
        draft: String,
    },
    Transition(String),
    Approve {
        key: String,
        repo: String,
        pr: u32,
    },
    ApproveAll(String),
    RequestChanges {
        key: String,
        pr: u32,
        text: String,
    },
    InlineComment {
        key: String,
        pr: u32,
        file: String,
        line: String,
        text: String,
    },
    PostComment {
        key: String,
        text: String,
    },
    ReturnMissingPr(String),
    ReturnThreads(String),
    AcceptThreads(String),
    ConflictComment {
        key: String,
        also_return: bool,
    },
    BreakLock(String),
    ExtendWait {
        key: String,
        kind: WaitKind,
    },
    Reclaim(String),
    Dismiss(String),
    Snooze {
        id: String,
        minutes: u32,
    },
    Undo(Undo),
    MarkSeen(String),
    SetProvider {
        provider: Provider,
        ready: bool,
    },
    SetWaitMinutes(u32),
    StartWorkspace,
    StopWorkspace,
    ResetDemo,
    Sim(SimEvent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffMode {
    Unified,
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinceMode {
    Since,
    Full,
}

/// What the text sheet is for.
#[derive(Clone, Debug, PartialEq)]
pub enum ComposeKind {
    RequestChanges { key: String, pr: u32 },
}

/// Every editable text in the app, so the session can own the text and the views stay stateless.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    TypedKey,
    Compose,
    Palette,
    Notes(String),
    Draft { key: String, id: String },
    Comment(String),
    Inline,
    Checklist(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    Go(Route),
    OpenTicket(String),
    CloseTab(String),
    ForceCloseTab(String),
    Diagnose,
    PinTab(String),
    ToggleAttention,
    ToggleSimulate,
    TogglePanel,
    ToggleShowAll,
    OpenPalette,
    ClosePalette,
    /// Send a command. If the store previews it, a confirm sheet opens first.
    Do(Command),
    /// The typed-key field and the confirm button of the open sheet.
    ConfirmSheet,
    CancelSheet,
    PickBaseline(Baseline),
    SetDiffMode(DiffMode),
    SetSinceMode {
        key: String,
        mode: SinceMode,
    },
    SelectPr {
        key: String,
        pr: u32,
    },
    SelectFile {
        key: String,
        pr: u32,
        index: usize,
    },
    /// `id` is the store's own key for the hunk; the session only remembers which ids are viewed.
    ToggleViewed {
        id: String,
    },
    InlineOpen {
        key: String,
        pr: u32,
        file: String,
        line: String,
    },
    InlineCancel,
    /// A text field changed. The session keeps the text; views render from the view model.
    SetText(Field, String),
    /// Send what was typed in a field (add a checklist item, post a comment, ...).
    Submit(Field),
    RequestChangesFrom {
        key: String,
        pr: u32,
    },
    Noop,
}

impl Intent {
    pub fn go_tickets(group: Group) -> Self {
        Intent::Go(Route::Tickets(group))
    }

    pub fn go_ticket(key: &str, tab: TicketTab) -> Self {
        Intent::Go(Route::ticket(key, tab))
    }
}

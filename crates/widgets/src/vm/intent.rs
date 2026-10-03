//! What a view can ask for. Views only ever emit an [`Intent`]; they never mutate anything.
//!
//! `Intent` is UI state (navigation, selections, sheets). `Command` is a request to the [`Store`](crate::Store):
//! the thing the engine will eventually do. A command the store previews is confirmed in a sheet first.

use super::common::{Group, MappingKey, Phase, Route, ThemeChoice, TicketTab};
use super::ids::*;
use super::screens::{TicketFilter, TicketSort};

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
    Conflict(Option<RepoName>),
    OverlayLeak(Option<RepoName>),
    UatMoves(bool),
    FailDeploy(Option<RepoName>),
    Mention(TicketKey),
    SignOff(TicketKey),
    Returned(TicketKey),
    BackToReview(TicketKey),
    ExternalLock(RepoName),
    StaleLock(RepoName),
    PrArrives(TicketKey),
    ResolveThreads(TicketKey),
    NewCommits(TicketKey),
    UatAfterPush(TicketKey),
    SkipMinutes(u32),
    /// Docker is not running: the services steps of a sequence fail.
    DockerDown(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Undo {
    Claim(TicketKey),
    /// Restore the review mark and the phase the ticket was in before it was marked reviewed.
    Reviewed {
        key: TicketKey,
        prev_mark: Option<u32>,
        prev_phase: Phase,
    },
    Response(SuggestionId),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Sync,
    Claim(TicketKey),
    StartReview(TicketKey),
    MarkReviewed(TicketKey),
    Activate {
        key: TicketKey,
        baseline: Option<Baseline>,
    },
    Park(TicketKey),
    /// Park the ticket in hand (`from`), then continue with `key`.
    ParkAndContinue {
        from: TicketKey,
        key: TicketKey,
        then: Then,
    },
    Recheck,
    ToggleChecklist {
        key: TicketKey,
        index: usize,
    },
    AddChecklist {
        key: TicketKey,
        text: String,
    },
    SetNotes {
        key: TicketKey,
        text: String,
    },
    Choose {
        key: TicketKey,
        repo: RepoName,
        branch: Branch,
    },
    Prepare(TicketKey),
    TogglePrePush {
        key: TicketKey,
        repo: RepoName,
        index: usize,
    },
    PushUat(TicketKey),
    Rerun {
        key: TicketKey,
        repo: RepoName,
    },
    ComposeDraft(TicketKey),
    EditDraft {
        key: TicketKey,
        id: DraftId,
        text: String,
    },
    PostAndMove {
        key: TicketKey,
        draft: DraftId,
    },
    Transition(TicketKey),
    Approve {
        key: TicketKey,
        repo: RepoName,
        pr: PrNumber,
    },
    ApproveAll(TicketKey),
    RequestChanges {
        key: TicketKey,
        pr: PrNumber,
        text: String,
    },
    InlineComment {
        key: TicketKey,
        pr: PrNumber,
        file: String,
        line: LineAnchor,
        text: String,
    },
    PostComment {
        key: TicketKey,
        text: String,
    },
    ReturnMissingPr(TicketKey),
    ReturnThreads(TicketKey),
    AcceptThreads(TicketKey),
    ConflictComment {
        key: TicketKey,
        also_return: bool,
    },
    BreakLock(RepoName),
    ExtendWait {
        key: TicketKey,
        kind: WaitKind,
    },
    Reclaim(TicketKey),
    Dismiss(SuggestionId),
    Snooze {
        id: SuggestionId,
        minutes: u32,
    },
    Undo(Undo),
    MarkSeen(TicketKey),
    SetProvider {
        provider: Provider,
        ready: bool,
    },
    SetWaitMinutes(u32),
    /// Minutes between automatic syncs; 0 turns them off.
    SetSyncInterval(u32),
    /// Save these Jira mapping settings together; empty text clears a setting back to its default.
    SaveMapping {
        changes: Vec<(MappingKey, String)>,
    },
    /// Clear every Jira mapping setting back to its default. Asks first.
    ResetMapping,
    /// Open this saved workspace: the tickets, repos and audit log shown are its own.
    SelectWorkspace(WorkspaceName),
    /// Close the open workspace, leaving none selected.
    CloseWorkspace,
    /// After a sequence with failures: carry on anyway (open the workspace, or close it) with the failures noted.
    ContinueSequence,
    /// After a sequence with failures: run it again from the top.
    RetrySequence,
    /// After a sequence with failures: give up; the workspace stays as it was.
    AbortSequence,
    StartWorkspace,
    StopWorkspace,
    /// Ask the engine again how the workspace's services stand (the screen checks by itself
    /// too, on open, after start and stop, and every so often while it is shown).
    RefreshHealth,
    /// Move every project of the open workspace to this branch. Projects that do not have it fall
    /// back to their own baseline, and a project with uncommitted changes is left alone and says
    /// why. It runs as a sequence of steps, one project at a time.
    SwitchBranch(Branch),
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
    RequestChanges { key: TicketKey, pr: PrNumber },
}

/// Every editable text in the app, so the session can own the text and the views stay stateless.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    TypedKey,
    Compose,
    Palette,
    Notes(TicketKey),
    Draft {
        key: TicketKey,
        id: DraftId,
    },
    Comment(TicketKey),
    Inline,
    Checklist(TicketKey),
    /// The filter box of the Next list.
    NextFilter,
    /// The search box of the ticket table.
    TicketFilter,
    /// The search box of the workspace menu.
    WorkspaceSearch,
    /// The search box of the branch picker.
    BranchSearch,
    /// One of the editable Jira mapping fields on the settings page.
    Mapping(MappingKey),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    Go(Route),
    OpenTicket(TicketKey),
    CloseTab(TicketKey),
    ForceCloseTab(TicketKey),
    Diagnose,
    /// Open or close the workspace menu of the title bar.
    ToggleWorkspacePicker,
    /// Open the modal that lists every saved workspace.
    OpenAllWorkspaces,
    /// Open the modal that lists every service of the open workspace and how each one stands.
    OpenServices,
    /// Open the modal that picks a branch for the whole workspace to move to.
    OpenBranchPicker,
    /// Save every edited Jira mapping field in one go.
    SaveMapping,
    /// Put the Jira mapping fields back to what is stored.
    DiscardMapping,
    /// Show this sync log (again: read it again, it may still be growing).
    SelectLog(String),
    PinTab(TicketKey),
    ToggleAttention,
    ToggleSimulate,
    TogglePanel,
    SetTheme(ThemeChoice),
    ToggleShowAll,
    ToggleTicketFilter(TicketFilter),
    ClearTicketFilters,
    /// Sort the ticket table by a column; again to flip the direction, a third time to clear.
    SortTickets(TicketSort),
    OpenPalette,
    ClosePalette,
    /// Send a command. A remote write always opens a confirm sheet first; some local ones do too.
    Do(Command),
    /// The typed-key field and the confirm button of the open sheet.
    ConfirmSheet,
    CancelSheet,
    PickBaseline(Baseline),
    /// Pick (or pick again) a branch in the branch picker; it then says what switching would do.
    PickBranch(Branch),
    SetDiffMode(DiffMode),
    SetSinceMode {
        key: TicketKey,
        mode: SinceMode,
    },
    SelectPr {
        key: TicketKey,
        pr: PrNumber,
    },
    SelectFile {
        key: TicketKey,
        pr: PrNumber,
        index: usize,
    },
    /// `id` is the store's own key for the hunk; the session only remembers which ids are viewed.
    ToggleViewed {
        id: HunkId,
    },
    InlineOpen {
        key: TicketKey,
        pr: PrNumber,
        file: String,
        line: LineAnchor,
    },
    InlineCancel,
    /// A text field changed. The session keeps the text; views render from the view model.
    SetText(Field, String),
    /// Send what was typed in a field (add a checklist item, post a comment, ...).
    Submit(Field),
    RequestChangesFrom {
        key: TicketKey,
        pr: PrNumber,
    },
    /// Copy raw text to the clipboard (branch, SHA, URL, command, log line). Handled by
    /// the app shell; the session records it and toasts.
    Copy(String),
    /// Open a URL in the browser. Handled by the app shell; the session records it.
    OpenExternal(String),
    Noop,
}

impl Intent {
    pub fn go_tickets(group: Group) -> Self {
        Intent::Go(Route::Tickets(group))
    }

    pub fn go_ticket(key: impl Into<TicketKey>, tab: TicketTab) -> Self {
        Intent::Go(Route::ticket(key, tab))
    }
}

impl Command {
    /// Commands that write to Jira, GitHub or a remote git branch. They can only reach the store
    /// through a [`Preview`](super::Preview) and [`Confirmed`](super::Confirmed).
    pub fn is_remote(&self) -> bool {
        matches!(
            self,
            Command::PushUat(_)
                | Command::Rerun { .. }
                | Command::PostAndMove { .. }
                | Command::Transition(_)
                | Command::Approve { .. }
                | Command::ApproveAll(_)
                | Command::RequestChanges { .. }
                | Command::InlineComment { .. }
                | Command::PostComment { .. }
                | Command::ReturnMissingPr(_)
                | Command::ReturnThreads(_)
                | Command::ConflictComment { .. }
        )
    }

    /// The single place a command is sorted into local or remote.
    pub fn classify(self) -> Classified {
        if self.is_remote() {
            Classified::Remote(RemoteCommand(self))
        } else {
            Classified::Local(LocalCommand(self))
        }
    }
}

/// A command that changes only local state.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalCommand(Command);

impl LocalCommand {
    pub fn command(&self) -> &Command {
        &self.0
    }
    pub fn into_command(self) -> Command {
        self.0
    }
}

/// A command that writes to a remote. Only constructible through [`Command::classify`], and only
/// executable as a [`Confirmed`](super::Confirmed).
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteCommand(Command);

impl RemoteCommand {
    pub fn command(&self) -> &Command {
        &self.0
    }
    pub fn into_command(self) -> Command {
        self.0
    }
}

pub enum Classified {
    Local(LocalCommand),
    Remote(RemoteCommand),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::{Preview, Risk, Unconfirmed};

    /// Exhaustive on purpose (no wildcard): adding a command forces a decision here about whether it is remote.
    fn expected_remote(c: &Command) -> bool {
        match c {
            Command::PushUat(_)
            | Command::Rerun { .. }
            | Command::PostAndMove { .. }
            | Command::Transition(_)
            | Command::Approve { .. }
            | Command::ApproveAll(_)
            | Command::RequestChanges { .. }
            | Command::InlineComment { .. }
            | Command::PostComment { .. }
            | Command::ReturnMissingPr(_)
            | Command::ReturnThreads(_)
            | Command::ConflictComment { .. } => true,
            Command::Sync
            | Command::Claim(_)
            | Command::StartReview(_)
            | Command::MarkReviewed(_)
            | Command::Activate { .. }
            | Command::Park(_)
            | Command::ParkAndContinue { .. }
            | Command::Recheck
            | Command::ToggleChecklist { .. }
            | Command::AddChecklist { .. }
            | Command::SetNotes { .. }
            | Command::Choose { .. }
            | Command::Prepare(_)
            | Command::TogglePrePush { .. }
            | Command::ComposeDraft(_)
            | Command::EditDraft { .. }
            | Command::AcceptThreads(_)
            | Command::BreakLock(_)
            | Command::ExtendWait { .. }
            | Command::Reclaim(_)
            | Command::Dismiss(_)
            | Command::Snooze { .. }
            | Command::Undo(_)
            | Command::MarkSeen(_)
            | Command::SetProvider { .. }
            | Command::SetWaitMinutes(_)
            | Command::SetSyncInterval(_)
            | Command::SaveMapping { .. }
            | Command::ResetMapping
            | Command::SelectWorkspace(_)
            | Command::CloseWorkspace
            | Command::ContinueSequence
            | Command::RetrySequence
            | Command::AbortSequence
            | Command::StartWorkspace
            | Command::StopWorkspace
            | Command::RefreshHealth
            | Command::SwitchBranch(_)
            | Command::ResetDemo
            | Command::Sim(_) => false,
        }
    }

    fn k(s: &str) -> TicketKey {
        s.into()
    }

    fn samples() -> Vec<Command> {
        let key = k("PROJ-1");
        vec![
            Command::PushUat(key.clone()),
            Command::Rerun {
                key: key.clone(),
                repo: "web".into(),
            },
            Command::PostAndMove {
                key: key.clone(),
                draft: "d".into(),
            },
            Command::Transition(key.clone()),
            Command::Approve {
                key: key.clone(),
                repo: "web".into(),
                pr: PrNumber(1),
            },
            Command::ApproveAll(key.clone()),
            Command::RequestChanges {
                key: key.clone(),
                pr: PrNumber(1),
                text: String::new(),
            },
            Command::InlineComment {
                key: key.clone(),
                pr: PrNumber(1),
                file: "f".into(),
                line: LineAnchor::New(1),
                text: String::new(),
            },
            Command::PostComment {
                key: key.clone(),
                text: String::new(),
            },
            Command::ReturnMissingPr(key.clone()),
            Command::ReturnThreads(key.clone()),
            Command::ConflictComment {
                key: key.clone(),
                also_return: false,
            },
            Command::Sync,
            Command::SaveMapping {
                changes: vec![(MappingKey::ReviewStatus, String::new())],
            },
            Command::SetSyncInterval(10),
            Command::SelectWorkspace("shop".into()),
            Command::CloseWorkspace,
            Command::ResetMapping,
            Command::Claim(key.clone()),
            Command::Park(key.clone()),
            Command::BreakLock("web".into()),
            Command::StartWorkspace,
            Command::RefreshHealth,
            Command::SwitchBranch("develop".into()),
            Command::Sim(SimEvent::Offline(true)),
            Command::Undo(Undo::Claim(key)),
        ]
    }

    #[test]
    fn classification_matches_the_single_source_of_truth() {
        for c in samples() {
            let want = expected_remote(&c);
            assert_eq!(c.is_remote(), want, "{c:?}");
            match c.clone().classify() {
                Classified::Remote(r) => assert!(want && r.command() == &c),
                Classified::Local(l) => assert!(!want && l.command() == &c),
            }
        }
    }

    #[test]
    fn ticket_keys_order_by_project_then_number() {
        let mut v = [k("PROJ-10"), k("PROJ-9"), k("ABC-100"), k("PROJ-2")];
        v.sort();
        let s: Vec<&str> = v.iter().map(|x| x.as_str()).collect();
        assert_eq!(s, ["ABC-100", "PROJ-2", "PROJ-9", "PROJ-10"]);
    }

    fn preview(risk: Risk) -> Preview {
        Preview::new(Command::PushUat(k("PROJ-1")), "Push", risk, "s", "Go")
    }

    #[test]
    fn a_blocked_preview_can_never_be_confirmed() {
        let mut p = preview(Risk::Low);
        p.blocked = Some("gh is signed out".into());
        assert!(!p.can_confirm(""));
        assert_eq!(
            p.confirm("").unwrap_err(),
            Unconfirmed::Blocked("gh is signed out".into())
        );
    }

    #[test]
    fn a_high_risk_preview_needs_its_key_typed() {
        let mut p = preview(Risk::High);
        p.type_key = Some("PROJ-1".into());
        assert!(!p.can_confirm("PROJ-2"));
        assert_eq!(
            p.clone().confirm("nope").unwrap_err(),
            Unconfirmed::WrongKey
        );
        let c = p.clone().confirm(" PROJ-1 ").expect("typed key accepted");
        assert_eq!(c.command(), p.command());
    }
}

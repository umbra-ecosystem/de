//! UI state on top of a [`Store`]: where you are, which tabs are open, which sheet is up, what you typed.
//!
//! The session is the only writer of UI state. Views emit [`Intent`]s into [`Session::handle`] and draw [`Session::view`].
//! It depends on no GPUI, so the whole interaction model is unit-tested headless (see the tests below).

use std::rc::Rc;
use std::collections::{BTreeMap, BTreeSet};

use crate::sim::Sim;
use crate::store::{Outcome, ReviewSel, Store};
use crate::vm::*;

#[derive(Clone, Debug)]
struct TabEntry {
    key: TicketKey,
    pinned: bool,
    tab: TicketTab,
}

#[derive(Clone, Debug)]
enum Sheet {
    Confirm {
        preview: Preview,
    },
    Baseline {
        key: TicketKey,
    },
    Report(ReportVm),
    Compose(ComposeKind),
    Palette,
    ClaimBlocked {
        key: TicketKey,
        block: ClaimBlock,
        then: Then,
    },
    Busy {
        busy: Busy,
        retry: Command,
    },
    Diagnose,
    CloseTab(TicketKey),
    /// The modal that lists every saved workspace.
    AllWorkspaces,
    /// The modal that picks a branch for the workspace to move to, and what is picked in it.
    Branches {
        chosen: Option<Branch>,
    },
}

const TOAST_SECS: f32 = 6.0;
const TOAST_UNDO_SECS: f32 = 8.0;

pub struct Session {
    store: Box<dyn Store>,
    route: Route,
    /// The last screen that was not a ticket: the first tab in the strip.
    screen: Route,
    tabs: Vec<TabEntry>,
    sheet: Option<Sheet>,
    toasts: Vec<Toast>,
    next_toast: u64,
    show_all: bool,
    ticket_filters: TicketFilters,
    ticket_sort: Option<(TicketSort, bool)>,
    attention_open: bool,
    /// The workspace menu of the title bar is open.
    picker_open: bool,
    simulate_open: bool,
    right_open: bool,
    theme: ThemeChoice,
    diff_mode: DiffMode,
    since: BTreeMap<TicketKey, SinceMode>,
    pr_sel: BTreeMap<TicketKey, PrNumber>,
    file_sel: BTreeMap<(TicketKey, PrNumber), usize>,
    viewed: BTreeSet<HunkId>,
    composer: Option<(TicketKey, PrNumber, String, LineAnchor)>,
    texts: BTreeMap<Field, String>,
    seen_snap: BTreeMap<TicketKey, u32>,
    /// The sync log being read: its id and lines, loaded once when selected.
    log_view: Option<(String, Rc<Vec<String>>)>,
    /// The mapping fields have been filled from the store. Edits then stay until saved or discarded, also when
    /// the user leaves the page and comes back.
    mapping_seeded: bool,
}

impl Session {
    pub fn new(store: Box<dyn Store>) -> Self {
        // With no workspace open there is nothing of its own to show: start on the list of workspaces.
        let start = if store.workspaces().current.is_some() {
            Route::Next
        } else {
            Route::Welcome
        };
        Self {
            store,
            route: start.clone(),
            screen: start,
            tabs: Vec::new(),
            sheet: None,
            toasts: Vec::new(),
            next_toast: 0,
            show_all: false,
            ticket_filters: TicketFilters::default(),
            ticket_sort: None,
            attention_open: false,
            picker_open: false,
            simulate_open: false,
            right_open: true,
            theme: ThemeChoice::System,
            diff_mode: DiffMode::Unified,
            since: BTreeMap::new(),
            pr_sel: BTreeMap::new(),
            file_sel: BTreeMap::new(),
            viewed: BTreeSet::new(),
            composer: None,
            texts: BTreeMap::new(),
            seen_snap: BTreeMap::new(),
            log_view: None,
            mapping_seeded: false,
        }
    }

    /// A session over the in-memory simulation, as the showcase uses it.
    pub fn demo() -> Self {
        Self::new(Box::new(Sim::new()))
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    pub fn store(&self) -> &dyn Store {
        self.store.as_ref()
    }

    pub fn text(&self, field: &Field) -> String {
        self.texts.get(field).cloned().unwrap_or_default()
    }

    pub fn has_sheet(&self) -> bool {
        self.sheet.is_some()
    }

    /* ---------------------------- time ---------------------------- */

    /// Advance by `millis` of real time. One simulated minute passes per four real seconds.
    pub fn tick(&mut self, millis: i64) {
        let before = self.store.workspaces().current;
        let toasts = self.store.tick(millis);
        // A sequence that has just finished opened or closed a workspace: nothing of the old one may linger.
        if self.store.workspaces().current != before {
            self.workspace_changed();
        }
        for (text, kind) in toasts {
            self.push_toast(text, kind, None);
        }
        let dt = millis as f32 / 1000.0;
        for t in &mut self.toasts {
            t.ttl -= dt;
        }
        self.toasts.retain(|t| t.ttl > 0.0);
    }

    fn push_toast(&mut self, text: String, kind: ToastKind, undo: Option<Intent>) {
        self.next_toast += 1;
        let ttl = if undo.is_some() {
            TOAST_UNDO_SECS
        } else {
            TOAST_SECS
        };
        self.toasts.push(Toast {
            id: self.next_toast,
            text,
            kind,
            undo,
            ttl,
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    /* ---------------------------- intents ---------------------------- */

    pub fn handle(&mut self, intent: Intent) {
        let before = self.store.workspaces().current;
        self.handle_inner(intent);
        // Carrying on past a failed sequence changes the workspace here, not on a tick.
        if self.store.workspaces().current != before {
            self.workspace_changed();
        }
    }

    fn handle_inner(&mut self, intent: Intent) {
        // Opening or closing a workspace is a modal nothing else can be done beside: only its own choices count,
        // and only once it has stopped running.
        if self.store.sequence().is_some()
            && !matches!(
                intent,
                Intent::Do(
                    Command::ContinueSequence | Command::RetrySequence | Command::AbortSequence
                )
                // The showcase's panel for the world outside is not the person using the app.
                | Intent::ToggleSimulate
                | Intent::Do(Command::Sim(_))
            )
        {
            return;
        }
        match intent {
            Intent::OpenAllWorkspaces => {
                self.picker_open = false;
                self.texts.remove(&Field::WorkspaceSearch);
                self.sheet = Some(Sheet::AllWorkspaces);
            }
            Intent::OpenBranchPicker => {
                self.texts.remove(&Field::BranchSearch);
                self.sheet = Some(Sheet::Branches { chosen: None });
            }
            Intent::PickBranch(branch) => {
                if let Some(Sheet::Branches { chosen }) = &mut self.sheet {
                    *chosen = Some(branch);
                }
            }
            Intent::Go(route) => self.go(route),
            Intent::OpenTicket(key) => self.go(Route::ticket(key, TicketTab::Overview)),
            Intent::CloseTab(key) => {
                if self.text(&Field::Comment(key.clone())).trim().is_empty() {
                    self.close_tab(&key);
                } else {
                    self.sheet = Some(Sheet::CloseTab(key));
                }
            }
            Intent::ForceCloseTab(key) => {
                self.texts.remove(&Field::Comment(key.clone()));
                self.sheet = None;
                self.close_tab(&key);
            }
            Intent::Diagnose => self.sheet = Some(Sheet::Diagnose),
            Intent::SelectLog(id) => self.select_log(id),
            Intent::ToggleWorkspacePicker => {
                self.picker_open = !self.picker_open;
                self.texts.remove(&Field::WorkspaceSearch);
                if self.picker_open {
                    self.attention_open = false;
                    self.sheet = None;
                }
            }
            Intent::SaveMapping => {
                let changes = self.mapping_edits();
                if !changes.is_empty() {
                    self.run(Command::SaveMapping { changes });
                    // Saved: show the stored form. A failed save leaves the edits where they are.
                    if self.mapping_edits().is_empty() {
                        self.seed_mapping();
                    }
                }
            }
            Intent::DiscardMapping => self.seed_mapping(),
            Intent::PinTab(key) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.key == key) {
                    t.pinned = true;
                }
            }
            Intent::ToggleAttention => self.attention_open = !self.attention_open,
            Intent::ToggleSimulate => self.simulate_open = !self.simulate_open,
            Intent::TogglePanel => self.right_open = !self.right_open,
            Intent::SetTheme(t) => self.theme = t,
            Intent::ToggleShowAll => self.show_all = !self.show_all,
            Intent::ToggleTicketFilter(f) => self.ticket_filters.toggle(f),
            Intent::ClearTicketFilters => {
                self.ticket_filters = TicketFilters::default();
                self.texts.remove(&Field::TicketFilter);
            }
            Intent::SortTickets(col) => {
                self.ticket_sort = match self.ticket_sort {
                    Some((c, true)) if c == col => Some((col, false)),
                    Some((c, false)) if c == col => None,
                    _ => Some((col, true)),
                }
            }
            Intent::OpenPalette => {
                self.texts.remove(&Field::Palette);
                self.sheet = Some(Sheet::Palette);
            }
            Intent::ClosePalette | Intent::CancelSheet => {
                self.sheet = None;
                self.texts.remove(&Field::TypedKey);
                self.texts.remove(&Field::Compose);
                self.texts.remove(&Field::BranchSearch);
            }
            Intent::Do(cmd) => {
                if !matches!(self.sheet, Some(Sheet::Confirm { .. })) {
                    self.sheet = None;
                }
                self.run(cmd);
            }
            Intent::ConfirmSheet => self.confirm(),
            Intent::PickBaseline(b) => {
                if let Some(Sheet::Baseline { key }) = self.sheet.take() {
                    self.run(Command::Activate {
                        key,
                        baseline: Some(b),
                    });
                }
            }
            Intent::SetDiffMode(m) => self.diff_mode = m,
            Intent::SetSinceMode { key, mode } => {
                self.since.insert(key, mode);
            }
            Intent::SelectPr { key, pr } => {
                self.pr_sel.insert(key, pr);
            }
            Intent::SelectFile { key, pr, index } => {
                self.pr_sel.insert(key.clone(), pr);
                self.file_sel.insert((key, pr), index);
            }
            Intent::ToggleViewed { id } => {
                if !self.viewed.remove(&id) {
                    self.viewed.insert(id);
                }
            }
            Intent::InlineOpen {
                key,
                pr,
                file,
                line,
            } => {
                self.texts.remove(&Field::Inline);
                self.composer = Some((key, pr, file, line));
            }
            Intent::InlineCancel => {
                self.composer = None;
                self.texts.remove(&Field::Inline);
            }
            Intent::SetText(field, text) => self.set_text(field, text),
            Intent::Submit(field) => self.submit(field),
            Intent::RequestChangesFrom { key, pr } => {
                self.texts.remove(&Field::Compose);
                self.sheet = Some(Sheet::Compose(ComposeKind::RequestChanges { key, pr }));
            }
            Intent::Noop => {}
        }
    }

    fn set_text(&mut self, field: Field, text: String) {
        match &field {
            Field::Notes(key) => {
                self.local(Command::SetNotes {
                    key: key.clone(),
                    text: text.clone(),
                });
            }
            Field::Draft { key, id } => {
                self.local(Command::EditDraft {
                    key: key.clone(),
                    id: id.clone(),
                    text: text.clone(),
                });
            }
            _ => {}
        }
        if text.is_empty() {
            self.texts.remove(&field);
        } else {
            self.texts.insert(field, text);
        }
    }

    fn submit(&mut self, field: Field) {
        let text = self.text(&field);
        match field {
            Field::Mapping(_) => self.handle(Intent::SaveMapping),
            Field::Checklist(key) => {
                if !text.trim().is_empty() {
                    self.local(Command::AddChecklist {
                        key: key.clone(),
                        text,
                    });
                    self.texts.remove(&Field::Checklist(key));
                }
            }
            Field::Comment(key) => {
                if !text.trim().is_empty() {
                    self.run(Command::PostComment { key, text });
                }
            }
            Field::Inline => {
                if text.trim().is_empty() {
                    return;
                }
                if let Some((key, pr, file, line)) = self.composer.clone() {
                    self.run(Command::InlineComment {
                        key,
                        pr,
                        file,
                        line,
                        text,
                    });
                }
            }
            Field::Compose => {
                if text.trim().is_empty() {
                    return;
                }
                if let Some(Sheet::Compose(ComposeKind::RequestChanges { key, pr })) =
                    self.sheet.take()
                {
                    self.texts.remove(&Field::Compose);
                    self.run(Command::RequestChanges { key, pr, text });
                }
            }
            _ => {}
        }
    }

    /// Fill the mapping fields from the store: the stored value, or the default when that is a plain value, so an
    /// unset field reads like a set one.
    fn seed_mapping(&mut self) {
        for row in self.store.settings().mapping {
            let text = if row.value.is_empty() {
                row.key.literal_default().unwrap_or_default().to_string()
            } else {
                row.value
            };
            self.set_text(Field::Mapping(row.key), text);
        }
        self.mapping_seeded = true;
    }

    /// The mapping fields whose text is not what is stored, with the value to save. Typing the default back
    /// means "not set".
    fn mapping_edits(&self) -> Vec<(MappingKey, String)> {
        self.store
            .settings()
            .mapping
            .into_iter()
            .filter_map(|row| {
                let mut text = self.text(&Field::Mapping(row.key)).trim().to_string();
                if row.key.literal_default() == Some(text.as_str()) {
                    text.clear();
                }
                (text != row.value).then_some((row.key, text))
            })
            .collect()
    }

    fn select_log(&mut self, id: String) {
        let text = self.store.log_text(&id);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        self.log_view = Some((id, Rc::new(lines)));
    }

    /// Whether a workspace is open.
    fn has_workspace(&self) -> bool {
        self.store.workspaces().current.is_some()
    }

    /// What the log screen says when it has no runs. A run belongs to the workspace it was made in,
    /// so without one the screen says so instead of showing runs nobody chose to see.
    fn logs_empty(&self) -> EmptyVm {
        if self.has_workspace() {
            EmptyVm::new(
                "No sync logs yet",
                "Every sync writes a raw log of what it asked Jira for and what came back. Run a sync and it appears here.",
            )
            .action(Btn::new("Sync now", Intent::Do(Command::Sync)).primary())
        } else {
            EmptyVm::new(
                "No workspace is open",
                "A sync run belongs to the workspace it was made in. Open a workspace and its runs are listed here.",
            )
            .action(Btn::new("Open a workspace", Intent::ToggleWorkspacePicker).primary())
        }
    }

    fn go(&mut self, route: Route) {
        // What belongs to a workspace cannot be shown without one: the list of workspaces is the way in.
        let route = if route.needs_workspace() && !self.has_workspace() {
            Route::Welcome
        } else {
            route
        };
        // The page before the dock is only for when no workspace is open; with one open, "Workspaces" is the menu.
        if route == Route::Welcome && self.has_workspace() {
            self.picker_open = true;
            self.texts.remove(&Field::WorkspaceSearch);
            return;
        }
        self.picker_open = false;
        if route == Route::Settings && !self.mapping_seeded {
            self.seed_mapping();
        }
        if route == Route::Logs
            && self.log_view.is_none()
            && let Some(first) = self.store.logs().0.first()
        {
            self.select_log(first.id.clone());
        }
        if self.sheet.is_some() && matches!(self.sheet, Some(Sheet::Palette)) {
            self.sheet = None;
        }
        self.attention_open = false;
        if let Route::Ticket { key, tab } = &route {
            if !self.store.ticket_exists(key) {
                return;
            }
            let entering = !matches!(&self.route, Route::Ticket { key: k, .. } if k == key);
            if entering {
                let seen = self.store.comments_seen(key);
                self.seen_snap.insert(key.clone(), seen);
                self.local(Command::MarkSeen(key.clone()));
            }
            match self.tabs.iter_mut().find(|t| &t.key == key) {
                Some(t) => t.tab = *tab,
                None => {
                    let entry = TabEntry {
                        key: key.clone(),
                        pinned: false,
                        tab: *tab,
                    };
                    match self.tabs.iter().position(|t| !t.pinned) {
                        Some(i) => self.tabs[i] = entry,
                        None => self.tabs.push(entry),
                    }
                }
            }
        } else {
            // Each list starts fresh: search, filters and sort belong to the list they were set on.
            if route != self.route {
                self.ticket_filters = TicketFilters::default();
                self.ticket_sort = None;
                self.texts.remove(&Field::TicketFilter);
            }
            self.screen = route.clone();
        }
        self.route = route;
    }

    fn close_tab(&mut self, key: &TicketKey) {
        self.tabs.retain(|t| t.key != *key);
        if matches!(&self.route, Route::Ticket { key: k, .. } if k == key) {
            self.route = self.screen.clone();
        }
    }

    /* ---------------------------- commands and sheets ---------------------------- */

    /// Dispatch a command that is known to be local (internal bookkeeping such as saving notes).
    fn local(&mut self, cmd: Command) -> Outcome {
        match cmd.classify() {
            Classified::Local(l) => self.store.dispatch(l),
            Classified::Remote(r) => Outcome::fail(format!(
                "{:?} writes to a remote and cannot be sent without a confirmation",
                r.command()
            )),
        }
    }

    /// The one entry point for a command. A remote write always opens its preview; a guarded local
    /// one does too; everything else is dispatched.
    fn run(&mut self, cmd: Command) {
        match cmd.clone().classify() {
            Classified::Remote(r) => match self.store.preview(&r) {
                Ok(preview) => self.open_confirm(preview),
                Err(why) => self.push_toast(why, ToastKind::Bad, None),
            },
            Classified::Local(l) => match self.store.local_guard(&l) {
                Some(preview) => self.open_confirm(preview),
                None => {
                    let out = self.store.dispatch(l);
                    self.apply(&cmd, out);
                }
            },
        }
    }

    fn open_confirm(&mut self, preview: Preview) {
        self.texts.remove(&Field::TypedKey);
        self.sheet = Some(Sheet::Confirm { preview });
    }

    /// The confirm button of the open sheet (and the dismiss button of a report).
    fn confirm(&mut self) {
        match self.sheet.take() {
            Some(Sheet::Confirm { preview }) => {
                let typed = self.text(&Field::TypedKey);
                let command = preview.command().clone();
                match preview.clone().confirm(&typed) {
                    Ok(confirmed) => {
                        self.texts.remove(&Field::TypedKey);
                        let out = self.store.execute(confirmed);
                        self.apply(&command, out);
                    }
                    // Blocked, or the key was not typed: the sheet stays as it was.
                    Err(_) => self.sheet = Some(Sheet::Confirm { preview }),
                }
            }
            Some(Sheet::Report(_)) | None => {}
            other => self.sheet = other,
        }
    }

    /// A different workspace is open (or none): nothing of the old one may linger in the window.
    fn workspace_changed(&mut self) {
        self.picker_open = false;
        self.attention_open = false;
        self.sheet = None;
        self.tabs.clear();
        self.show_all = false;
        self.ticket_filters = TicketFilters::default();
        self.ticket_sort = None;
        self.since.clear();
        self.pr_sel.clear();
        self.file_sel.clear();
        self.viewed.clear();
        self.composer = None;
        self.seen_snap.clear();
        // The run that was open belonged to the workspace that is gone.
        self.log_view = None;
        // Only settings text is not about a workspace.
        self.texts.retain(|f, _| matches!(f, Field::Mapping(_)));
        let start = if self.has_workspace() {
            Route::Next
        } else {
            Route::Welcome
        };
        self.route = start.clone();
        self.screen = start;
    }

    fn apply(&mut self, cmd: &Command, out: Outcome) {
        if matches!(cmd, Command::ResetMapping) && out.is_done() {
            self.seed_mapping();
        }
        match out {
            Outcome::NeedsBaseline => {
                if let Command::Activate { key, .. } = cmd {
                    self.sheet = Some(Sheet::Baseline { key: key.clone() });
                }
                return;
            }
            Outcome::Blocked(block) => {
                let (key, then) = match cmd {
                    Command::Claim(k) => (k.clone(), Then::Claim),
                    Command::StartReview(k) => (k.clone(), Then::StartReview),
                    Command::Reclaim(k) => (k.clone(), Then::Reclaim),
                    _ => return,
                };
                self.sheet = Some(Sheet::ClaimBlocked { key, block, then });
                return;
            }
            Outcome::Busy(busy) => {
                self.sheet = Some(Sheet::Busy {
                    busy,
                    retry: cmd.clone(),
                });
                return;
            }
            Outcome::Refused(errors) => {
                self.push_toast(errors.join(" "), ToastKind::Bad, None);
                return;
            }
            Outcome::Done { toast, report } => {
                if let Some(report) = report {
                    self.sheet = Some(Sheet::Report(report));
                }
                if let Some(t) = toast {
                    self.push_toast(t.text, t.kind, t.undo.map(|u| Intent::Do(Command::Undo(u))));
                }
            }
        }
        match cmd {
            Command::StartReview(key) => {
                self.go(Route::ticket(key.clone(), TicketTab::Review));
            }
            // An active ticket is there to be tested: open its Test tab.
            Command::Activate { key, .. } => {
                self.go(Route::ticket(key.clone(), TicketTab::Test));
            }
            Command::ParkAndContinue {
                key,
                then: Then::StartReview,
                ..
            } => {
                self.go(Route::ticket(key.clone(), TicketTab::Review));
            }
            Command::PostComment { key, .. } => {
                self.texts.remove(&Field::Comment(key.clone()));
            }
            Command::InlineComment { .. } => {
                self.composer = None;
                self.texts.remove(&Field::Inline);
            }
            Command::PostAndMove { key, .. } => {
                self.texts
                    .retain(|f, _| !matches!(f, Field::Draft { key: k, .. } if k == key));
            }
            Command::ResetDemo => {
                self.tabs.clear();
                self.route = Route::Next;
                self.screen = Route::Next;
                self.viewed.clear();
                self.composer = None;
                self.texts.clear();
                self.seen_snap.clear();
            }
            _ => {}
        }
    }

    /* ---------------------------- the view model ---------------------------- */

    fn review_sel(&self, key: &TicketKey) -> ReviewSel {
        let pr = self.pr_sel.get(key).copied();
        let file = pr
            .and_then(|p| self.file_sel.get(&(key.clone(), p)).copied())
            .unwrap_or(0);
        ReviewSel {
            pr,
            file,
            mode: self.diff_mode,
            since: self.since.get(key).copied().unwrap_or(SinceMode::Since),
            viewed: self.viewed.clone(),
            composer: self
                .composer
                .as_ref()
                .filter(|(k, ..)| k == key)
                .map(|(_, pr, f, l)| (*pr, f.clone(), *l)),
        }
    }

    fn nav(&self, counts: &Counts) -> Vec<NavSection> {
        let item = |label: &str, route: Route, count: usize| NavItem {
            selected: self.route == route,
            label: label.to_string(),
            route,
            count,
        };
        let list = |g: Group| item(g.label(), Route::Tickets(g), counts.group(g));
        if !self.has_workspace() {
            // Nothing of a workspace to navigate: the way in, and what does not need one.
            return vec![
                NavSection {
                    title: String::new(),
                    items: vec![item("Workspaces", Route::Welcome, 0)],
                },
                NavSection {
                    title: "System".to_string(),
                    items: vec![
                        item("Sync logs", Route::Logs, 0),
                        item("Settings", Route::Settings, 0),
                    ],
                },
            ];
        }
        vec![
            NavSection {
                title: String::new(),
                items: vec![
                    item("Home", Route::Next, counts.suggestions),
                    list(Group::Pool),
                    list(Group::All),
                ],
            },
            NavSection {
                title: "Status".to_string(),
                items: Group::NAV
                    .iter()
                    .filter(|g| **g != Group::Pool)
                    .map(|g| list(*g))
                    .collect(),
            },
            NavSection {
                title: "Environment".to_string(),
                items: vec![
                    item("On uat", Route::OnUat, 0),
                    item("Workspace", Route::Workspace, 0),
                ],
            },
            NavSection {
                title: "System".to_string(),
                items: vec![
                    item("Audit log", Route::Audit, 0),
                    item("Sync logs", Route::Logs, 0),
                    item("Settings", Route::Settings, 0),
                ],
            },
        ]
    }

    fn tab_strip(&self) -> Vec<ShellTab> {
        let label = |r: &Route| match r {
            Route::Next => "Home".to_string(),
            Route::Tickets(g) => g.label().to_string(),
            Route::OnUat => "On uat".to_string(),
            Route::Workspace => "Workspace".to_string(),
            Route::Audit => "Audit log".to_string(),
            Route::Logs => "Sync logs".to_string(),
            Route::Settings => "Settings".to_string(),
            Route::Welcome => "Workspaces".to_string(),
            Route::Ticket { key, .. } => key.to_string(),
        };
        let mut out = vec![ShellTab {
            route: self.screen.clone(),
            key: None,
            label: label(&self.screen),
            title: String::new(),
            selected: !matches!(self.route, Route::Ticket { .. }),
            pinned: true,
            bad: false,
            unseen: false,
            active: false,
        }];
        for t in &self.tabs {
            let Some(info) = self.store.tab_info(&t.key) else {
                continue;
            };
            let here = matches!(&self.route, Route::Ticket { key, .. } if *key == t.key);
            out.push(ShellTab {
                route: Route::ticket(t.key.clone(), t.tab),
                key: Some(t.key.clone()),
                label: t.key.to_string(),
                title: info.title,
                selected: here,
                pinned: t.pinned,
                bad: info.bad,
                unseen: info.unseen && !here,
                active: info.active,
            });
        }
        out
    }

    fn sheet_vm(&self) -> Option<SheetVm> {
        Some(match self.sheet.as_ref()? {
            Sheet::Confirm { preview } => {
                let typed = self.text(&Field::TypedKey);
                let can = preview.can_confirm(&typed);
                SheetVm::Confirm {
                    preview: preview.clone(),
                    typed,
                    can_confirm: can,
                }
            }
            Sheet::Baseline { key } => SheetVm::Baseline(self.store.baseline_choices(key)),
            Sheet::Report(r) => SheetVm::Report(r.clone()),
            Sheet::Compose(kind) => match kind {
                ComposeKind::RequestChanges { key, .. } => SheetVm::Compose {
                    kind: kind.clone(),
                    title: format!("Request changes on {key}"),
                    placeholder: "What has to change before this can be approved?".to_string(),
                    confirm_label: "Review comment…".to_string(),
                    text: self.text(&Field::Compose),
                },
            },
            Sheet::ClaimBlocked { key, block, then } => SheetVm::ClaimBlocked {
                key: key.clone(),
                block: block.clone(),
                then: *then,
            },
            Sheet::Busy { busy, retry } => SheetVm::Busy {
                busy: busy.clone(),
                retry: retry.clone(),
            },
            Sheet::Diagnose => SheetVm::Diagnose(self.store.diagnose()),
            Sheet::CloseTab(key) => SheetVm::CloseTab(key.clone()),
            Sheet::AllWorkspaces => SheetVm::AllWorkspaces(
                self.store
                    .all_workspaces(&self.text(&Field::WorkspaceSearch)),
            ),
            Sheet::Branches { chosen } => SheetVm::Branches(self.store.branch_picker(
                &self.text(&Field::BranchSearch),
                chosen.as_ref(),
            )),
            Sheet::Palette => {
                let query = self.text(&Field::Palette);
                SheetVm::Palette {
                    items: self.store.palette(&query),
                    query,
                }
            }
        })
    }

    /// Apply the search, filters and sort of the ticket table (shared by every ticket list) and list what the
    /// filter menu may offer, from the rows before they are narrowed.
    fn narrow_tickets(&self, v: &mut TicketListVm) {
        let q = self.text(&Field::TicketFilter).trim().to_lowercase();
        let mut options = TicketFilterOptions::default();
        for r in v.sections.iter().flat_map(|s| &s.rows) {
            options.repos.extend(r.repos.iter().cloned());
            options.jira.push(r.jira.text.clone());
            options.priority.push(r.priority.text.clone());
        }
        for f in [&mut options.jira, &mut options.priority] {
            f.sort();
            f.dedup();
        }
        options.repos.sort();
        options.repos.dedup();
        // What is chosen stays listed even if nothing is left to pick it from.
        for r in &self.ticket_filters.repos {
            if !options.repos.contains(r) {
                options.repos.push(r.clone());
            }
        }
        let filters = self.ticket_filters.clone();
        let sort = self.ticket_sort;
        for s in &mut v.sections {
            s.rows.retain(|r| {
                filters.keeps(r)
                    && (q.is_empty()
                        || r.title.to_lowercase().contains(&q)
                        || r.key.to_lowercase().contains(&q)
                        || r.sub.to_lowercase().contains(&q))
            });
            if let Some((col, asc)) = sort {
                s.rows.sort_by(|a, b| {
                    let o = match col {
                        TicketSort::Key => a.key.cmp(&b.key),
                        TicketSort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                        TicketSort::Jira => a.jira.text.cmp(&b.jira.text),
                    };
                    if asc { o } else { o.reverse() }
                });
            }
        }
        v.sections.retain(|s| !s.rows.is_empty());
        // Rows exist but the search or filters hide them all: say so, and offer the way back.
        if v.sections.is_empty() && v.total > 0 {
            v.empty = EmptyVm::new(
                "No ticket matches",
                "Nothing here fits the search and filters you have set.",
            )
            .action(Btn::new("Clear search and filters", Intent::ClearTicketFilters).primary());
        }
        v.options = options;
        v.filters = filters;
        v.sort = sort;
    }

    fn screen_vm(&self) -> ScreenVm {
        if self.route.needs_workspace() && !self.has_workspace() {
            return ScreenVm::Welcome(self.store.welcome());
        }
        match &self.route {
            Route::Welcome => ScreenVm::Welcome(self.store.welcome()),
            Route::Next => {
                let mut n = self.store.next(self.show_all);
                let q = self.text(&Field::NextFilter).trim().to_lowercase();
                if !q.is_empty() {
                    n.cards.retain(|c| {
                        c.title.to_lowercase().contains(&q)
                            || c.headline.as_ref().is_some_and(|h| h.to_lowercase().contains(&q))
                            || c.reason.to_lowercase().contains(&q)
                            || c.ticket
                                .as_ref()
                                .is_some_and(|k| k.to_lowercase().contains(&q))
                    });
                }
                if n.cards.is_empty() && !q.is_empty() {
                    n.empty = EmptyVm::new(
                        "No suggestion matches",
                        format!("Nothing matches \u{201c}{q}\u{201d}."),
                    )
                    .action(
                        Btn::new(
                            "Clear search",
                            Intent::SetText(Field::NextFilter, String::new()),
                        )
                        .primary(),
                    );
                }
                ScreenVm::Next(n)
            }
            Route::Tickets(g) => {
                let mut v = self.store.tickets(*g);
                self.narrow_tickets(&mut v);
                ScreenVm::Tickets(v)
            }
            Route::OnUat => {
                let mut v = self.store.on_uat();
                self.narrow_tickets(&mut v.table);
                ScreenVm::OnUat(v)
            }
            Route::Workspace => ScreenVm::Workspace(self.store.workspace()),
            Route::Audit => ScreenVm::Audit(self.store.audit()),
            Route::Logs => {
                let (runs, note) = self.store.logs();
                let shown = self
                    .log_view
                    .as_ref()
                    .filter(|(id, _)| runs.iter().any(|r| r.id == *id));
                ScreenVm::Logs(LogsVm {
                    selected: shown.map(|(id, _)| id.clone()),
                    lines: shown.map(|(_, l)| l.clone()).unwrap_or_default(),
                    runs,
                    note,
                    empty: self.logs_empty(),
                })
            }
            Route::Settings => {
                let mut v = self.store.settings();
                v.edited = self.mapping_edits().into_iter().map(|(k, _)| k).collect();
                v.appearance = ThemeChoice::LIST
                    .iter()
                    .map(|t| {
                        (
                            t.label().to_string(),
                            *t == self.theme,
                            Intent::SetTheme(*t),
                        )
                    })
                    .collect();
                ScreenVm::Settings(v)
            }
            Route::Ticket { key, tab } => {
                let Some(head) = self.store.ticket_head(key, *tab) else {
                    return ScreenVm::Missing(format!("{key} is not in the cache."));
                };
                let body = match tab {
                    TicketTab::Overview => self
                        .store
                        .overview(key, self.seen_snap.get(key).copied())
                        .map(TicketBody::Overview),
                    TicketTab::Review => self
                        .store
                        .review(key, &self.review_sel(key))
                        .map(TicketBody::Review),
                    TicketTab::Test => self.store.test(key).map(TicketBody::Test),
                    TicketTab::Ship => self.store.ship(key).map(TicketBody::Ship),
                    TicketTab::Timeline => Some(TicketBody::Timeline(self.store.timeline(key))),
                };
                match body {
                    Some(body) => ScreenVm::Ticket {
                        head: Box::new(head),
                        tab: *tab,
                        body: Box::new(body),
                        comment: self.text(&Field::Comment(key.clone())),
                        checklist_add: self.text(&Field::Checklist(key.clone())),
                    },
                    None => ScreenVm::Missing(format!("{key} is not in the cache.")),
                }
            }
        }
    }

    pub fn view(&self) -> AppVm {
        let counts = self.store.counts();
        let right = self.store.right_panel(&self.route);
        AppVm {
            route: self.route.clone(),
            nav: self.nav(&counts),
            tabs: self.tab_strip(),
            status: self.store.status(),
            workspace: self.workspace_chip(),
            picker: self.picker_open.then(|| self.picker()),
            attention_count: counts.attention,
            attention: self.attention_open.then(|| self.store.attention()),
            simulate: self.simulate_open.then(|| self.store.simulate()),
            right_title: match &self.route {
                Route::Ticket { .. } => "Details",
                Route::Next => "Today",
                _ => "Activity",
            }
            .to_string(),
            right_open: self.right_open && !right.is_empty(),
            right,
            theme: self.theme,
            screen: self.screen_vm(),
            sheet: self.sheet_vm(),
            sequence: self.store.sequence(),
            toasts: self.toasts.clone(),
        }
    }

    fn workspace_chip(&self) -> WorkspaceChip {
        match self.store.workspaces().current {
            Some(name) => WorkspaceChip {
                label: name.to_string(),
                none: false,
            },
            None => WorkspaceChip {
                label: "No workspace".to_string(),
                none: true,
            },
        }
    }

    /// The workspace menu: the open one, then the others most recently used first, narrowed by the search.
    fn picker(&self) -> PickerVm {
        let all = self.store.workspaces();
        let query = self.text(&Field::WorkspaceSearch);
        let q = query.trim().to_lowercase();
        let matches = |w: &WorkspaceItemVm| q.is_empty() || w.name.to_lowercase().contains(&q);
        let total = all.items.len();
        let (current, recent): (Vec<_>, Vec<_>) = all
            .items
            .into_iter()
            .filter(|w| matches(w))
            .partition(|w| w.current);
        PickerVm {
            query,
            current: current.into_iter().next(),
            recent,
            total,
            init_command: self.store.welcome().init_command,
        }
    }

    /// Texts the views must mirror into their inputs (used to clear an input after a send).
    pub fn field_texts(&self) -> &BTreeMap<Field, String> {
        &self.texts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> TicketKey {
        TicketKey::from(s)
    }

    fn go_ticket(s: &mut Session, key: &str, tab: TicketTab) {
        s.handle(Intent::Go(Route::ticket(key, tab)));
    }

    fn next_titles(s: &Session) -> Vec<String> {
        s.store()
            .next(false)
            .cards
            .iter()
            .map(|c| c.title.clone())
            .collect()
    }

    fn sheet_kind(s: &Session) -> &'static str {
        match s.sheet {
            None => "none",
            Some(Sheet::Confirm { .. }) => "confirm",
            Some(Sheet::Baseline { .. }) => "baseline",
            Some(Sheet::Report(_)) => "report",
            Some(Sheet::Compose(_)) => "compose",
            Some(Sheet::Palette) => "palette",
            Some(Sheet::ClaimBlocked { .. }) => "claim-blocked",
            Some(Sheet::Busy { .. }) => "busy",
            Some(Sheet::Diagnose) => "diagnose",
            Some(Sheet::CloseTab(_)) => "close-tab",
            Some(Sheet::AllWorkspaces) => "all-workspaces",
            Some(Sheet::Branches { .. }) => "branches",
        }
    }

    /// PROJ-142 starts out claimed (as in the prototype); tests that need a free hand release it.
    fn free_hand(s: &mut Session) {
        s.handle(Intent::Do(Command::Undo(Undo::Claim("PROJ-142".into()))));
    }

    fn confirm_with(s: &mut Session, typed: Option<&str>) {
        if let Some(t) = typed {
            s.handle(Intent::SetText(Field::TypedKey, t.to_string()));
        }
        s.handle(Intent::ConfirmSheet);
    }

    #[test]
    fn next_leads_with_the_hotfix_then_actions_then_what_is_only_information() {
        // The ordering rules and their tests live in `sim/ranking.rs`; this is the view from the session.
        let s = Session::demo();
        let titles = next_titles(&s);
        assert_eq!(
            titles[0], "Claim PROJ-139",
            "the hotfix can be claimed even with a ticket in hand"
        );
        assert_eq!(titles[1], "Start the review");
        assert!(
            titles[2..].iter().all(|t| t.contains("mentioned")),
            "only information is left after the actions: {titles:?}"
        );
    }

    #[test]
    fn one_ticket_at_a_time_then_park_and_continue() {
        let mut s = Session::demo();
        assert_eq!(s.store().counts().group(Group::Mine), 1);
        s.handle(Intent::Do(Command::Claim("PROJ-150".into())));
        assert_eq!(sheet_kind(&s), "claim-blocked");
        assert_eq!(
            s.store().counts().group(Group::Mine),
            1,
            "second claim refused"
        );
        s.handle(Intent::Do(Command::ParkAndContinue {
            from: "PROJ-142".into(),
            key: "PROJ-150".into(),
            then: Then::Claim,
        }));
        assert_eq!(sheet_kind(&s), "none");
        assert_eq!(s.store().counts().group(Group::Parked), 1);
        assert_eq!(s.store().counts().group(Group::Mine), 1);
    }

    #[test]
    fn claim_can_be_undone_from_the_toast() {
        let mut s = Session::demo();
        free_hand(&mut s);
        s.handle(Intent::Do(Command::Claim("PROJ-150".into())));
        assert_eq!(s.store().counts().group(Group::Mine), 1);
        let undo = s
            .toasts
            .last()
            .and_then(|t| t.undo.clone())
            .expect("undo offered");
        s.handle(undo);
        assert_eq!(s.store().counts().group(Group::Mine), 0);
    }

    #[test]
    fn toasts_expire() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::StartReview("PROJ-142".into())));
        s.handle(Intent::Do(Command::MarkReviewed("PROJ-142".into())));
        assert_eq!(s.toasts.len(), 1);
        s.tick(9000);
        assert!(s.toasts.is_empty());
    }

    #[test]
    fn preview_tabs_are_replaced_until_pinned() {
        let mut s = Session::demo();
        go_ticket(&mut s, "PROJ-142", TicketTab::Overview);
        go_ticket(&mut s, "PROJ-150", TicketTab::Overview);
        assert_eq!(s.tabs.len(), 1);
        s.handle(Intent::PinTab("PROJ-150".into()));
        go_ticket(&mut s, "PROJ-142", TicketTab::Overview);
        assert_eq!(s.tabs.len(), 2);
        s.handle(Intent::CloseTab("PROJ-142".into()));
        assert_eq!(s.route, Route::Next);
    }

    #[test]
    fn new_comments_are_marked_until_the_tab_is_reopened() {
        let mut s = Session::demo();
        // PROJ-131 has a mention we have not seen
        go_ticket(&mut s, "PROJ-131", TicketTab::Overview);
        let ScreenVm::Ticket { body, .. } = s.view().screen else {
            panic!()
        };
        let TicketBody::Overview(o) = *body else {
            panic!()
        };
        assert!(o.comments.iter().any(|c| c.is_new));
        // seen now: the tab strip no longer flags it after leaving
        s.handle(Intent::Go(Route::Next));
        assert!(!s.store().tab_info(&k("PROJ-131")).unwrap().unseen);
    }

    #[test]
    fn reviewing_a_ticket_with_a_missing_pr_is_blocked() {
        let mut s = Session::demo();
        free_hand(&mut s);
        s.handle(Intent::Do(Command::Claim("PROJ-163".into())));
        s.handle(Intent::Do(Command::StartReview("PROJ-163".into())));
        assert!(s.toasts.iter().any(|t| t.text.contains("No pull request")));
        let head = s
            .store()
            .ticket_head(&k("PROJ-163"), TicketTab::Overview)
            .unwrap();
        assert!(
            head.banners
                .iter()
                .any(|b| b.title.contains("Missing pull request"))
        );
    }

    #[test]
    fn a_hotfix_asks_for_a_baseline() {
        let mut s = Session::demo();
        free_hand(&mut s);
        for c in [
            Command::Claim("PROJ-139".into()),
            Command::StartReview("PROJ-139".into()),
            Command::MarkReviewed("PROJ-139".into()),
        ] {
            s.handle(Intent::Do(c));
        }
        s.handle(Intent::Do(Command::Activate {
            key: "PROJ-139".into(),
            baseline: None,
        }));
        assert_eq!(sheet_kind(&s), "baseline");
        s.handle(Intent::PickBaseline(Baseline::Develop));
        assert_eq!(sheet_kind(&s), "report");
        assert_eq!(s.store().counts().group(Group::Active), 1);
    }

    /// The whole story of a ticket, through the session only: claim to approved.
    #[test]
    fn a_ticket_from_claim_to_approved() {
        let mut s = Session::demo();
        let key = &k("PROJ-142");
        for c in [
            Command::StartReview(key.into()),
            Command::MarkReviewed(key.into()),
        ] {
            s.handle(Intent::Do(c));
        }
        // the open thread on PROJ-142 is resolved, so activation is allowed
        s.handle(Intent::Do(Command::Activate {
            key: key.into(),
            baseline: None,
        }));
        assert_eq!(sheet_kind(&s), "report");
        s.handle(Intent::CancelSheet);
        let Some(TestVm::Active { overlay, .. }) = s.store().test(key) else {
            panic!("active")
        };
        assert!(
            overlay.is_some(),
            "web consumes api-client, so the overlay applies"
        );

        for i in 0..3 {
            s.handle(Intent::Do(Command::ToggleChecklist {
                key: key.into(),
                index: i,
            }));
        }
        s.handle(Intent::Do(Command::Prepare(key.into())));
        // the pre-push checks lock the push button
        let ShipVm {
            integrate: IntegrateBody::Active { push, .. },
            ..
        } = s.store().ship(key).unwrap()
        else {
            panic!()
        };
        assert!(!push.enabled);
        for (repo, n) in [("api-client", 2), ("web", 2)] {
            for index in 0..n {
                s.handle(Intent::Do(Command::TogglePrePush {
                    key: key.into(),
                    repo: repo.into(),
                    index,
                }));
            }
        }
        let ShipVm {
            integrate: IntegrateBody::Active { push, .. },
            ..
        } = s.store().ship(key).unwrap()
        else {
            panic!()
        };
        assert!(push.enabled);

        // high risk: the sheet wants the ticket key typed
        s.handle(Intent::Do(Command::PushUat(key.into())));
        assert_eq!(sheet_kind(&s), "confirm");
        confirm_with(&mut s, Some("wrong"));
        assert_eq!(sheet_kind(&s), "confirm", "wrong key does not confirm");
        confirm_with(&mut s, Some(key));
        assert_eq!(sheet_kind(&s), "none");
        assert_eq!(
            s.store().counts().group(Group::Awaiting),
            2,
            "PROJ-142 joined 127 (131 is returned)"
        );

        // the runs take a few simulated minutes
        s.tick(9000);
        let ship = s.store().ship(key).unwrap();
        assert!(matches!(ship.announce, AnnounceBody::Compose(_)));
        s.handle(Intent::Do(Command::ComposeDraft(key.into())));
        let ShipVm {
            announce: AnnounceBody::Draft { id, text, .. },
            ..
        } = s.store().ship(key).unwrap()
        else {
            panic!()
        };
        assert!(text.contains("Deployed to alpha"));
        assert!(!text.contains("Comments while testing"));

        // editing the draft is what gets posted
        s.handle(Intent::SetText(
            Field::Draft {
                key: key.into(),
                id: id.clone(),
            },
            "Edited draft".into(),
        ));
        s.handle(Intent::Do(Command::PostAndMove {
            key: key.into(),
            draft: id,
        }));
        assert_eq!(sheet_kind(&s), "confirm");
        let SheetVm::Confirm { preview, .. } = s.view().sheet.unwrap() else {
            panic!()
        };
        assert!(preview.payload.iter().any(|l| l.contains("Edited draft")));
        confirm_with(&mut s, None);
        let head = s.store().ticket_head(key, TicketTab::Overview).unwrap();
        assert_eq!(head.jira.text, JIRA_ALPHA_LABEL);

        // approval is only offered after sign-off
        let review = s.store().review(key, &s.review_sel(key)).unwrap();
        assert!(!review.approve.enabled);
        s.handle(Intent::Do(Command::Sim(SimEvent::SignOff(key.into()))));
        let review = s.store().review(key, &s.review_sel(key)).unwrap();
        assert!(review.approve.enabled);
        s.handle(Intent::Do(Command::ApproveAll(key.into())));
        confirm_with(&mut s, None);
        assert_eq!(s.store().counts().group(Group::Done), 1);
    }

    const JIRA_ALPHA_LABEL: &str = "Alpha Testing";

    #[test]
    fn a_suggestion_row_opens_its_ticket_and_only_ticket_rows_are_clickable() {
        let s = Session::demo();
        let cards = s.store().next(true).cards;
        assert!(!cards.is_empty());
        for c in &cards {
            assert_eq!(c.open.is_some(), c.ticket.is_some(), "{}", c.title);
        }
        let start = cards
            .iter()
            .find(|c| c.title == "Start the review")
            .expect("the demo suggests starting a review");
        assert_eq!(
            start.open,
            Some(Intent::go_ticket("PROJ-142", TicketTab::Review))
        );
    }

    #[test]
    fn activating_a_ticket_opens_its_test_tab() {
        let mut s = Session::demo();
        let key = k("PROJ-142");
        for c in [
            Command::StartReview(key.clone()),
            Command::MarkReviewed(key.clone()),
            Command::Activate {
                key: key.clone(),
                baseline: None,
            },
        ] {
            s.handle(Intent::Do(c));
        }
        assert_eq!(s.route, Route::ticket("PROJ-142", TicketTab::Test));
    }

    #[test]
    fn announcing_waits_while_the_ticket_is_being_re_integrated() {
        let mut s = Session::demo();
        // PROJ-131 shipped once and came back; activating it starts a re-merge.
        s.handle(Intent::Do(Command::Activate {
            key: k("PROJ-131"),
            baseline: None,
        }));
        s.handle(Intent::CancelSheet);
        go_ticket(&mut s, "PROJ-131", TicketTab::Ship);
        let ScreenVm::Ticket { body, .. } = s.view().screen else {
            panic!("expected a ticket screen")
        };
        let TicketBody::Ship(ship) = *body else {
            panic!("expected the Ship tab")
        };
        // PROJ-131 is active and was announced for an earlier merge: nothing may be sent until the re-merge lands.
        assert_eq!(ship.integrate_state, StepState::Now);
        let AnnounceBody::Posted {
            transition: Some(go),
            ..
        } = ship.announce
        else {
            panic!("the earlier announcement is shown")
        };
        assert!(!go.enabled && go.hint.is_some());
        let Classified::Remote(r) = Command::Transition(k("PROJ-131")).classify() else {
            panic!("a transition is remote")
        };
        assert!(s.store().preview(&r).is_err());
    }

    #[test]
    fn a_failed_deploy_offers_a_rerun_through_the_confirm_sheet() {
        let mut s = Session::demo();
        let key = &k("PROJ-142");
        for c in [
            Command::StartReview(key.into()),
            Command::MarkReviewed(key.into()),
            Command::Activate {
                key: key.into(),
                baseline: None,
            },
        ] {
            s.handle(Intent::Do(c));
        }
        s.handle(Intent::CancelSheet);
        s.handle(Intent::Do(Command::Sim(SimEvent::FailDeploy(Some(
            "web".into(),
        )))));
        for i in 0..3 {
            s.handle(Intent::Do(Command::ToggleChecklist {
                key: key.into(),
                index: i,
            }));
        }
        s.handle(Intent::Do(Command::Prepare(key.into())));
        for (repo, n) in [("api-client", 2), ("web", 2)] {
            for index in 0..n {
                s.handle(Intent::Do(Command::TogglePrePush {
                    key: key.into(),
                    repo: repo.into(),
                    index,
                }));
            }
        }
        s.handle(Intent::Do(Command::PushUat(key.into())));
        confirm_with(&mut s, Some(key));
        s.tick(9000);
        let ship = s.store().ship(key).unwrap();
        let web = ship.runs.iter().find(|r| r.repo == "web").unwrap();
        assert_eq!(web.chip.tone, Tone::Bad);
        assert!(web.failure.is_some());
        assert!(
            next_titles(&s)
                .iter()
                .any(|t| t.contains("Deploy failed in web"))
        );
        s.handle(web.rerun.clone().unwrap().intent);
        assert_eq!(sheet_kind(&s), "confirm");
        confirm_with(&mut s, None);
        s.tick(9000);
        assert!(
            s.store()
                .ship(key)
                .unwrap()
                .runs
                .iter()
                .all(|r| r.chip.tone == Tone::Ok)
        );
    }

    #[test]
    fn a_signed_out_tool_blocks_the_confirm_sheet() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::SetProvider {
            provider: Provider::GitHub,
            ready: false,
        }));
        s.handle(Intent::Do(Command::Rerun {
            key: "PROJ-127".into(),
            repo: "web".into(),
        }));
        let SheetVm::Confirm {
            preview,
            can_confirm,
            ..
        } = s.view().sheet.unwrap()
        else {
            panic!()
        };
        assert!(preview.blocked.is_some());
        assert!(!can_confirm);
        // and Next says so instead of offering the write
        s.handle(Intent::CancelSheet);
    }

    #[test]
    fn a_conflict_with_uat_blocks_the_push_and_offers_a_comment() {
        let mut s = Session::demo();
        let key = &k("PROJ-142");
        for c in [
            Command::StartReview(key.into()),
            Command::MarkReviewed(key.into()),
            Command::Sim(SimEvent::Conflict(Some("web".into()))),
        ] {
            s.handle(Intent::Do(c));
        }
        let review = s.store().review(key, &s.review_sel(key)).unwrap();
        assert_eq!(review.uat.as_ref().map(|b| b.tone), Some(Tone::Bad));
        assert!(
            next_titles(&s)
                .iter()
                .any(|t| t.contains("Conflict with uat in web"))
        );
        s.handle(Intent::Do(Command::ConflictComment {
            key: key.into(),
            also_return: false,
        }));
        assert_eq!(sheet_kind(&s), "confirm");
        confirm_with(&mut s, None);
        assert!(
            !next_titles(&s)
                .iter()
                .any(|t| t.contains("Conflict with uat in web"))
        );
    }

    #[test]
    fn dismissed_suggestions_come_back_when_the_facts_change() {
        let mut s = Session::demo();
        let id = s.store().next(false).cards[0].id.clone();
        s.handle(Intent::Do(Command::Dismiss(id.clone())));
        assert!(s.store().next(false).cards.iter().all(|c| c.id != id));
        assert_eq!(
            s.store()
                .next(true)
                .cards
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.state),
            Some(SugState::Dismissed)
        );
        let undo = s.toasts.last().and_then(|t| t.undo.clone()).unwrap();
        s.handle(undo);
        assert!(s.store().next(false).cards.iter().any(|c| c.id == id));
    }

    #[test]
    fn sync_brings_scripted_arrivals() {
        let mut s = Session::demo();
        assert!(!s.store().ticket_exists(&k("PROJ-155")));
        s.handle(Intent::Do(Command::Sync));
        assert!(s.store().status().sync_running);
        s.tick(2000);
        assert!(s.store().ticket_exists(&k("PROJ-155")));
        assert!(!s.store().status().sync_running);
    }

    #[test]
    fn offline_sync_keeps_the_cache_and_says_so() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::Sim(SimEvent::Offline(true))));
        s.handle(Intent::Do(Command::Sync));
        s.tick(2000);
        assert!(!s.store().ticket_exists(&k("PROJ-155")));
        assert_eq!(s.store().status().sync_tone, Tone::Warn);
    }

    #[test]
    fn a_stale_lock_refuses_activation_and_can_be_removed() {
        let mut s = Session::demo();
        let key = &k("PROJ-142");
        for c in [
            Command::StartReview(key.into()),
            Command::MarkReviewed(key.into()),
            Command::Sim(SimEvent::StaleLock("web".into())),
            Command::Activate {
                key: key.into(),
                baseline: None,
            },
        ] {
            s.handle(Intent::Do(c));
        }
        assert_eq!(sheet_kind(&s), "busy");
        let SheetVm::Busy { busy, .. } = s.view().sheet.unwrap() else {
            panic!()
        };
        assert!(busy.stale && busy.message.contains("leftover .git/index.lock"));
        assert_eq!(s.store().counts().group(Group::Active), 0);
        s.handle(Intent::Do(Command::BreakLock("web".into())));
        confirm_with(&mut s, None);
        s.handle(Intent::Do(Command::Activate {
            key: key.into(),
            baseline: None,
        }));
        assert_eq!(s.store().counts().group(Group::Active), 1);
    }

    #[test]
    fn request_changes_goes_through_the_compose_sheet_then_confirm() {
        let mut s = Session::demo();
        s.handle(Intent::RequestChangesFrom {
            key: "PROJ-150".into(),
            pr: PrNumber(490),
        });
        assert_eq!(sheet_kind(&s), "compose");
        s.handle(Intent::SetText(Field::Compose, "Keep the logo row".into()));
        s.handle(Intent::Submit(Field::Compose));
        assert_eq!(sheet_kind(&s), "confirm");
        confirm_with(&mut s, None);
        let t = s
            .store()
            .review(&k("PROJ-150"), &s.review_sel(&k("PROJ-150")))
            .unwrap();
        assert!(t.thread_count >= 2);
    }

    #[test]
    fn inline_comments_need_a_line_and_a_confirm() {
        let mut s = Session::demo();
        s.handle(Intent::InlineOpen {
            key: "PROJ-142".into(),
            pr: PrNumber(212),
            file: "src/redirect.rs".into(),
            line: LineAnchor::New(41),
        });
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        assert!(r.composer.is_some());
        s.handle(Intent::SetText(
            Field::Inline,
            "Why not reuse landing()?".into(),
        ));
        s.handle(Intent::Submit(Field::Inline));
        assert_eq!(sheet_kind(&s), "confirm");
        confirm_with(&mut s, None);
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        assert!(r.composer.is_none());
        assert!(r.thread_count >= 2);
    }

    #[test]
    fn viewed_hunks_collapse() {
        let mut s = Session::demo();
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        let before = r.hunks[0].rows.len();
        assert!(before > 0);
        s.handle(r.hunks[0].toggle.clone());
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        assert!(r.hunks[0].viewed);
        assert!(r.hunks[0].rows.is_empty());
    }

    #[test]
    fn new_commits_mark_the_review_stale_and_offer_the_since_view() {
        let mut s = Session::demo();
        for c in [
            Command::StartReview("PROJ-142".into()),
            Command::MarkReviewed("PROJ-142".into()),
            Command::Sim(SimEvent::NewCommits("PROJ-142".into())),
        ] {
            s.handle(Intent::Do(c));
        }
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        assert!(r.stale.is_some());
        assert!(r.since_toggle.is_some());
        assert!(next_titles(&s).iter().any(|t| t.contains("Re-review")));
        s.handle(Intent::SetSinceMode {
            key: "PROJ-142".into(),
            mode: SinceMode::Full,
        });
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        assert_eq!(r.since_toggle.map(|t| t.2), Some(false));
    }

    #[test]
    fn split_mode_pairs_deletions_with_additions() {
        let mut s = Session::demo();
        s.handle(Intent::SetDiffMode(DiffMode::Split));
        let r = s
            .store()
            .review(&k("PROJ-142"), &s.review_sel(&k("PROJ-142")))
            .unwrap();
        assert!(r.hunks[0].rows.iter().any(|row| matches!(
            row,
            DiffRow::Pair {
                left: Some(_),
                right: Some(_),
                ..
            }
        )));
    }

    #[test]
    fn the_palette_finds_tickets_and_navigates() {
        let mut s = Session::demo();
        s.handle(Intent::OpenPalette);
        s.handle(Intent::SetText(Field::Palette, "onboarding".into()));
        let SheetVm::Palette { items, .. } = s.view().sheet.unwrap() else {
            panic!()
        };
        assert_eq!(items.len(), 1);
        s.handle(items[0].intent.clone());
        assert_eq!(sheet_kind(&s), "none");
        assert!(matches!(s.route, Route::Ticket { .. }));
    }

    #[test]
    fn reset_returns_to_the_start() {
        let mut s = Session::demo();
        let before = s.store().counts();
        free_hand(&mut s);
        s.handle(Intent::Do(Command::Claim("PROJ-150".into())));
        go_ticket(&mut s, "PROJ-150", TicketTab::Review);
        s.handle(Intent::Do(Command::ResetDemo));
        assert_eq!(s.route, Route::Next);
        assert_eq!(s.store().counts(), before);
        assert!(s.tabs.is_empty());
    }

    #[test]
    fn a_busy_repo_opens_the_busy_sheet_and_retry_works_once_free() {
        let mut s = Session::demo();
        for c in [
            Command::StartReview("PROJ-142".into()),
            Command::MarkReviewed("PROJ-142".into()),
            Command::Sim(SimEvent::ExternalLock("api-client".into())),
            Command::Activate {
                key: "PROJ-142".into(),
                baseline: None,
            },
        ] {
            s.handle(Intent::Do(c));
        }
        assert_eq!(sheet_kind(&s), "busy");
        s.handle(Intent::Do(Command::Sim(SimEvent::ExternalLock(
            "api-client".into(),
        ))));
        let SheetVm::Busy { retry, .. } =
            s.view().sheet.unwrap_or(SheetVm::Diagnose(String::new()))
        else {
            // the sheet was closed by the Do above; retrying is what the button does
            s.handle(Intent::Do(Command::Activate {
                key: "PROJ-142".into(),
                baseline: None,
            }));
            assert_eq!(s.store().counts().group(Group::Active), 1);
            return;
        };
        s.handle(Intent::Do(retry));
        assert_eq!(s.store().counts().group(Group::Active), 1);
    }

    #[test]
    fn closing_a_tab_with_an_unsent_comment_asks_first() {
        let mut s = Session::demo();
        go_ticket(&mut s, "PROJ-150", TicketTab::Overview);
        s.handle(Intent::SetText(
            Field::Comment("PROJ-150".into()),
            "half a thought".into(),
        ));
        s.handle(Intent::CloseTab("PROJ-150".into()));
        assert_eq!(sheet_kind(&s), "close-tab");
        s.handle(Intent::ForceCloseTab("PROJ-150".into()));
        assert_eq!(s.route, Route::Next);
        assert!(s.tabs.is_empty());
        assert!(s.text(&Field::Comment("PROJ-150".into())).is_empty());
    }

    #[test]
    fn a_claim_suggestion_states_its_facts_as_chips() {
        let s = Session::demo();
        let ScreenVm::Next(n) = s.view().screen else {
            panic!()
        };
        let card = n
            .cards
            .iter()
            .find(|c| c.title == "Claim PROJ-139")
            .expect("the hotfix claim");
        let texts: Vec<&str> = card.chips.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"Hotfix"), "{texts:?}");
        // Priority is a mark beside the key and the place in the queue is the order: neither is a chip.
        assert!(!texts.iter().any(|t| t.contains("queue") || ["Highest", "High", "Medium", "Low"].contains(t)), "{texts:?}");
        // The sentence is still there for search, and other kinds of row keep showing it.
        assert!(card.reason.contains("Review column"));
        assert!(n.cards.iter().any(|c| c.chips.is_empty()));
        // A claim row is about the ticket: its own title on the first line, type and assignee on the second.
        let t = Session::demo();
        let demo_ticket = t.store().tab_info(&"PROJ-139".into()).expect("ticket");
        assert_eq!(card.headline.as_deref(), Some(demo_ticket.title.as_str()));
        assert!(card.meta.as_deref().is_some_and(|m| !m.is_empty()));
        // Other rows keep their own title and sentence.
        assert!(n.cards.iter().any(|c| c.headline.is_none() && c.meta.is_none()));
        // Only a high priority gets the mark.
        assert!(n.cards.iter().all(|c| c
            .priority
            .as_ref()
            .is_none_or(|p| p.text == "High" || p.text == "Highest")));
        assert!(n.cards.iter().any(|c| c.priority.is_some()));
    }

    #[test]
    fn searching_the_home_list_finds_a_claim_row_by_its_ticket_title() {
        let mut s = Session::demo();
        let title = {
            let ScreenVm::Next(n) = s.view().screen else {
                panic!()
            };
            n.cards
                .iter()
                .find(|c| c.title == "Claim PROJ-139")
                .and_then(|c| c.headline.clone())
                .unwrap()
        };
        let word = title.split_whitespace().next().unwrap().to_string();
        s.handle(Intent::SetText(Field::NextFilter, word.to_lowercase()));
        let ScreenVm::Next(n) = s.view().screen else {
            panic!()
        };
        assert!(n.cards.iter().any(|c| c.title == "Claim PROJ-139"));
    }

    #[test]
    fn the_claim_queue_in_the_side_panel_is_one_clickable_line_per_ticket() {
        let s = Session::demo();
        let right = s.view().right;
        let queue = right
            .iter()
            .find(|sec| sec.title == "Claim queue")
            .expect("the claim queue");
        let tickets: Vec<(&TicketKey, &Option<Badge>)> = queue
            .rows
            .iter()
            .filter_map(|r| match r {
                RightRow::Ticket { key, mark, .. } => Some((key, mark)),
                _ => None,
            })
            .collect();
        assert!(!tickets.is_empty());
        assert_eq!(tickets.len(), queue.rows.len(), "nothing but ticket lines");
        // Only a hotfix or a high priority is marked.
        for (_, mark) in &tickets {
            if let Some(m) = mark {
                assert!(["Hotfix", "High", "Highest"].contains(&m.text.as_str()), "{}", m.text);
            }
        }
        assert!(tickets.iter().any(|(_, m)| m.is_some()) && tickets.iter().any(|(_, m)| m.is_none()));
    }

    #[test]
    fn the_sync_interval_is_a_choice_on_the_settings_page() {
        let mut s = Session::demo();
        let selected = |s: &Session| {
            s.store()
                .settings()
                .sync_options
                .into_iter()
                .filter(|(_, on, _)| *on)
                .map(|(label, _, _)| label)
                .collect::<Vec<_>>()
        };
        assert_eq!(selected(&s), ["10m"]);
        s.handle(Intent::Do(Command::SetSyncInterval(30)));
        assert_eq!(selected(&s), ["30m"]);
        assert!(s.view().toasts.iter().any(|t| t.text.contains("every 30 minutes")));
        s.handle(Intent::Do(Command::SetSyncInterval(0)));
        assert_eq!(selected(&s), ["Off"]);
        assert!(s.view().toasts.iter().any(|t| t.text.contains("off")));
        let labels: Vec<String> = s.store().settings().sync_options.into_iter().map(|o| o.0).collect();
        assert_eq!(labels, ["Off", "5m", "10m", "30m", "1h"]);
    }

    /// Let the opening or closing sequence run until it has finished or needs a decision.
    fn settle(s: &mut Session) {
        for _ in 0..400 {
            match s.store().sequence() {
                None => return,
                Some(q) if q.state == SequenceState::NeedsDecision => return,
                Some(_) => s.tick(500),
            }
        }
        panic!("the sequence did not finish");
    }

    /// Select a workspace and wait for it to be open.
    fn open_ws(s: &mut Session, name: &str) {
        s.handle(Intent::Do(Command::SelectWorkspace(name.into())));
        settle(s);
    }

    fn names(w: &WorkspacesVm) -> Vec<String> {
        w.items.iter().map(|i| i.name.to_string()).collect()
    }

    fn ticket_keys(s: &Session) -> Vec<String> {
        s.store()
            .tickets(Group::All)
            .sections
            .into_iter()
            .flat_map(|sec| sec.rows)
            .map(|r| r.key.to_string())
            .collect()
    }

    #[test]
    fn saved_workspaces_are_listed_most_recently_used_first_with_the_open_one_on_top() {
        let mut s = Session::demo();
        let w = s.store().workspaces();
        assert_eq!(w.current.as_ref().map(|n| n.as_str()), Some("shop"));
        assert_eq!(names(&w), ["shop", "threadplay", "hbt", "quelle", "orbit", "pair"]);
        assert_eq!(w.items[0].last_used, "open");
        assert_eq!(w.items[1].last_used, "3h ago");
        assert_eq!(w.items.last().unwrap().last_used, "never");

        open_ws(&mut s, "hbt");
        let w = s.store().workspaces();
        assert_eq!(names(&w), ["hbt", "shop", "threadplay", "quelle", "orbit", "pair"], "hbt was just used, shop just left");
        assert_eq!(w.items[1].last_used, "0m ago".replace("0m ago", "just now"));
    }

    #[test]
    fn a_workspace_shows_only_its_own_data_and_gets_it_back_as_it_was() {
        let mut s = Session::demo();
        let shop = ticket_keys(&s);
        assert!(shop.iter().any(|k| k == "PROJ-142"));

        open_ws(&mut s, "threadplay");
        let tp = ticket_keys(&s);
        assert_eq!(tp, ["TP-12", "TP-18", "TP-15"], "its own, by priority: none of shop's tickets");
        assert!(!s.store().ticket_exists(&k("PROJ-142")));
        assert!(s.store().audit().iter().all(|a| a.ticket.as_ref().is_none_or(|t| t.starts_with("TP-"))));

        // Something done in threadplay stays in threadplay.
        s.handle(Intent::Do(Command::Claim(k("TP-15"))));
        assert!(s.store().audit().iter().any(|a| a.action == "ticket.claim"));

        open_ws(&mut s, "shop");
        assert_eq!(ticket_keys(&s), shop);
        assert!(
            !s.store().audit().iter().any(|a| a.ticket.as_ref().is_some_and(|t| t.as_str() == "TP-15")),
            "threadplay's audit entries are not in shop's"
        );
        open_ws(&mut s, "threadplay");
        let claimed = s.store().tickets(Group::Mine).sections.into_iter().flat_map(|x| x.rows).any(|r| r.key == "TP-15");
        assert!(claimed, "threadplay kept the claim");
    }

    #[test]
    fn the_workspaces_start_and_stop_independently() {
        let mut s = Session::demo();
        assert!(s.store().workspace().up);
        open_ws(&mut s, "hbt");
        assert!(!s.store().workspace().up, "hbt was never started");
        assert_eq!(s.store().workspace().name, "hbt");
        s.handle(Intent::Do(Command::StartWorkspace));
        assert!(s.store().workspace().up);
        open_ws(&mut s, "shop");
        assert!(s.store().workspace().up);
        s.handle(Intent::Do(Command::StopWorkspace));
        open_ws(&mut s, "hbt");
        assert!(s.store().workspace().up, "stopping shop did not stop hbt");
    }

    /// The env screen's services line is an exception: silence while everything runs, one
    /// badge when it does not, and Refresh asks the engine again either way.
    #[test]
    fn the_services_line_is_an_exception_and_refresh_asks_the_engine_again() {
        let mut s = Session::demo();
        // All running: nothing to say, no badge.
        assert_eq!(s.store().workspace().health, None);

        // A check that could not run is the loudest thing on the screen, with what to do.
        s.handle(Intent::Do(Command::Sim(SimEvent::DockerDown(true))));
        let health = s
            .store()
            .workspace()
            .health
            .clone()
            .expect("a failed check is said out loud");
        assert_eq!(health.badge().text, "Docker unavailable");
        assert!(
            health.notice().unwrap().contains("Start Docker Desktop"),
            "the notice says what to do next"
        );

        // Refresh asks again: with Docker still down it says so instead of pretending.
        s.handle(Intent::Do(Command::RefreshHealth));
        assert!(
            s.view()
                .toasts
                .iter()
                .any(|t| t.text.contains("Docker is not running")),
            "{:?}",
            s.view().toasts
        );

        // Docker back: refresh clears the exception back to silence, and says it checked.
        s.handle(Intent::Do(Command::Sim(SimEvent::DockerDown(false))));
        s.handle(Intent::Do(Command::RefreshHealth));
        assert_eq!(s.store().workspace().health, None);
        assert!(
            s.view().toasts.iter().any(|t| t.text == "Services checked"),
            "{:?}",
            s.view().toasts
        );

        // A stopped workspace is one exception at the workspace level; each row counts its own.
        // Stopping is guarded: it asks first, and only the confirmed stop changes anything.
        s.handle(Intent::Do(Command::StopWorkspace));
        assert!(
            matches!(s.view().sheet, Some(SheetVm::Confirm { .. })),
            "stopping the workspace asks first"
        );
        s.handle(Intent::ConfirmSheet);
        let v = s.store().workspace();
        assert!(
            matches!(
                v.health,
                Some(HealthVm::Stopped { running: 0, declared }) if declared > 0
            ),
            "{:?}",
            v.health
        );
        assert!(
            v.rows
                .iter()
                .any(|r| matches!(&r.services, ServicesVm::Partial { running: 0, .. })),
            "each row says how many of its services are stopped"
        );
    }

    #[test]
    fn opening_another_workspace_leaves_nothing_of_the_old_one_in_the_window() {
        let mut s = Session::demo();
        s.handle(Intent::OpenTicket(k("PROJ-142")));
        s.handle(Intent::SetText(Field::Comment(k("PROJ-142")), "half written".into()));
        s.handle(Intent::Go(Route::Tickets(Group::Mine)));
        s.handle(Intent::OpenTicket(k("PROJ-139")));
        assert!(s.view().tabs.len() > 1);
        s.handle(Intent::Go(Route::Logs));
        s.handle(Intent::SelectLog("sync-run-2.log".into()));

        open_ws(&mut s, "threadplay");
        let v = s.view();
        assert_eq!(v.route, Route::Next);
        assert_eq!(v.tabs.len(), 1, "no ticket tabs of the old workspace");
        assert!(v.sheet.is_none() && v.picker.is_none());
        assert_eq!(s.text(&Field::Comment(k("PROJ-142"))), "", "no draft of the old one");
        assert_eq!(v.workspace.label, "threadplay");

        // The run that was open belonged to the workspace that was open.
        s.handle(Intent::Go(Route::Logs));
        let ScreenVm::Logs(v) = s.view().screen else {
            panic!("not the log screen")
        };
        assert_eq!(
            v.selected.as_deref(),
            Some("sync-run-0.log"),
            "the run of the old workspace is forgotten; the newest of this one is open"
        );
    }

    #[test]
    fn with_no_workspace_only_the_workspace_list_the_logs_and_settings_are_reachable() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        assert_eq!(s.route(), &Route::Welcome);
        let v = s.view();
        assert!(v.workspace.none);
        assert_eq!(v.workspace.label, "No workspace");
        let labels: Vec<String> = v.nav.iter().flat_map(|sec| sec.items.iter().map(|i| i.label.clone())).collect();
        assert_eq!(labels, ["Workspaces", "Sync logs", "Settings"]);

        // What belongs to a workspace leads back to the list.
        for route in [Route::Next, Route::Tickets(Group::All), Route::OnUat, Route::Workspace, Route::Audit] {
            s.handle(Intent::Go(route));
            assert_eq!(s.route(), &Route::Welcome);
            assert!(matches!(s.view().screen, ScreenVm::Welcome(_)));
        }
        s.handle(Intent::Go(Route::Settings));
        assert_eq!(s.route(), &Route::Settings);
        s.handle(Intent::Go(Route::Logs));
        assert_eq!(s.route(), &Route::Logs);

        // The palette has no tickets to jump to, but can switch workspace.
        s.handle(Intent::OpenPalette);
        let SheetVm::Palette { items, .. } = s.view().sheet.unwrap() else {
            panic!()
        };
        assert!(items.iter().all(|i| i.hint != "ticket"));
        assert!(items.iter().all(|i| i.label != "Home" && i.label != "On uat" && i.label != "Audit log"));
        assert!(items.iter().any(|i| i.label == "Open shop"));
        assert!(items.iter().any(|i| i.label == "Workspaces"));
    }

    fn states(q: &SequenceVm) -> Vec<ProgressState> {
        q.phases.iter().flat_map(|p| p.steps.iter().map(|s| s.state)).collect()
    }

    #[test]
    fn selecting_a_workspace_runs_visible_steps_and_only_then_opens_it() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        s.handle(Intent::Do(Command::SelectWorkspace("hbt".into())));

        // Nothing has changed yet; the modal is up.
        assert!(s.view().workspace.none);
        let q = s.view().sequence.expect("the opening sequence");
        assert_eq!(q.title, "Opening hbt");
        assert_eq!(q.state, SequenceState::Running);
        assert!(q.actions.is_empty(), "no choices while it runs");
        let labels: Vec<&str> = q.phases[0].steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Read the workspace",
                "Git status \u{b7} hbt-api",
                "Git status \u{b7} hbt-web",
                "Git status \u{b7} hbt-docs",
                "Start services \u{b7} hbt-api",
                "Start services \u{b7} hbt-web",
                "Load the workspace",
            ],
            "git for every repo first, then services only for repos that have them"
        );
        assert_eq!(q.progress, "0 of 7");

        // A little time: the first step is done, the second is running, the rest wait.
        s.tick(500);
        let q = s.view().sequence.unwrap();
        assert_eq!(q.phases[0].steps[0].state, ProgressState::Done);
        assert_eq!(q.phases[0].steps[0].detail, "3 projects");
        assert_eq!(q.phases[0].steps[1].state, ProgressState::Running);
        assert_eq!(q.phases[0].steps[2].state, ProgressState::Waiting);
        assert!(s.view().workspace.none, "still not open");

        settle(&mut s);
        assert!(s.view().sequence.is_none());
        assert_eq!(s.view().workspace.label, "hbt");
        assert_eq!(s.route(), &Route::Next, "the dock, with the workspace loaded");
        assert!(s.view().toasts.iter().any(|t| t.text == "Opened hbt"));
    }

    #[test]
    fn while_a_sequence_runs_nothing_else_in_the_window_works_and_it_cannot_be_dismissed() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        s.handle(Intent::Do(Command::SelectWorkspace("hbt".into())));
        s.tick(500);

        for intent in [
            Intent::CancelSheet,
            Intent::Go(Route::Settings),
            Intent::OpenPalette,
            Intent::ToggleWorkspacePicker,
            Intent::OpenAllWorkspaces,
            Intent::Do(Command::SelectWorkspace("shop".into())),
            Intent::Do(Command::CloseWorkspace),
            Intent::Do(Command::AbortSequence),
            Intent::Do(Command::ContinueSequence),
            Intent::Do(Command::RetrySequence),
        ] {
            s.handle(intent.clone());
            let q = s.view().sequence.unwrap_or_else(|| panic!("{intent:?} closed the modal"));
            assert_eq!(q.state, SequenceState::Running, "{intent:?}");
            assert_eq!(q.title, "Opening hbt", "{intent:?}");
            assert_eq!(s.route(), &Route::Welcome, "{intent:?}");
            assert!(s.view().sheet.is_none() && s.view().picker.is_none(), "{intent:?}");
        }
        // And what it was doing carried on regardless.
        assert!(states(&s.view().sequence.unwrap()).iter().any(|st| *st == ProgressState::Done));
    }

    #[test]
    fn a_failed_service_start_leaves_the_decision_to_the_person() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        s.handle(Intent::Do(Command::Sim(SimEvent::DockerDown(true))));
        s.handle(Intent::Do(Command::SelectWorkspace("hbt".into())));
        settle(&mut s);

        let q = s.view().sequence.expect("still up, waiting for a decision");
        assert_eq!(q.state, SequenceState::NeedsDecision);
        let failed: Vec<&str> = q
            .phases[0]
            .steps
            .iter()
            .filter(|st| st.state == ProgressState::Failed)
            .map(|st| st.detail.as_str())
            .collect();
        assert_eq!(failed.len(), 2, "both docker steps failed, the rest still ran");
        assert!(failed.iter().all(|d| d.contains("Start Docker Desktop")), "{failed:?}");
        assert_eq!(q.phases[0].steps.last().unwrap().state, ProgressState::Done, "loading went on");
        let labels: Vec<&str> = q.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Back", "Try again", "Open anyway"]);
        assert!(s.view().workspace.none, "not open until decided");

        // Try again with Docker back: everything is fine and it opens by itself.
        s.handle(Intent::Do(Command::Sim(SimEvent::DockerDown(false))));
        assert!(s.view().sequence.is_some(), "the world did not move the modal");
        s.handle(Intent::Do(Command::RetrySequence));
        let q = s.view().sequence.unwrap();
        assert_eq!(q.state, SequenceState::Running);
        assert!(states(&q).iter().all(|st| matches!(st, ProgressState::Waiting | ProgressState::Running)), "from the top");
        settle(&mut s);
        assert_eq!(s.view().workspace.label, "hbt");
    }

    #[test]
    fn back_gives_up_and_open_anyway_goes_on_with_the_failures() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        s.handle(Intent::Do(Command::Sim(SimEvent::DockerDown(true))));

        s.handle(Intent::Do(Command::SelectWorkspace("hbt".into())));
        settle(&mut s);
        s.handle(Intent::Do(Command::AbortSequence));
        assert!(s.view().sequence.is_none());
        assert!(s.view().workspace.none, "back: nothing was opened");
        assert_eq!(s.route(), &Route::Welcome);

        s.handle(Intent::Do(Command::SelectWorkspace("hbt".into())));
        settle(&mut s);
        s.handle(Intent::Do(Command::ContinueSequence));
        assert!(s.view().sequence.is_none());
        assert_eq!(s.view().workspace.label, "hbt", "open anyway");
        assert_eq!(s.route(), &Route::Next);
    }

    #[test]
    fn switching_closes_the_open_workspace_first_in_the_same_modal() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::SelectWorkspace("hbt".into())));
        let q = s.view().sequence.expect("a switch");
        assert_eq!(q.title, "Switching to hbt");
        assert_eq!(q.phases.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), ["Closing shop", "Opening hbt"]);
        assert_eq!(s.view().workspace.label, "shop", "shop stays open until its own closing is done");
        let closing: Vec<&str> = q.phases[0].steps.iter().map(|st| st.label.as_str()).collect();
        assert_eq!(closing[0], "Check nothing is in progress");
        assert_eq!(*closing.last().unwrap(), "Save the workspace");
        assert!(closing.iter().any(|l| l.starts_with("Stop services")), "{closing:?}");
        // The services are stopped in the reverse of the order they start.
        let stops: Vec<&&str> = closing.iter().filter(|l| l.starts_with("Stop services")).collect();
        let starts_of_shop: Vec<String> = {
            let mut order: Vec<String> = Sim::default().repos.iter().filter(|r| r.services > 0).map(|r| r.name.to_string()).collect();
            order.reverse();
            order
        };
        assert_eq!(stops.len(), starts_of_shop.len());
        for (stop, repo) in stops.iter().zip(&starts_of_shop) {
            assert!(stop.ends_with(repo.as_str()), "{stop} vs {repo}");
        }

        settle(&mut s);
        assert_eq!(s.view().workspace.label, "hbt");
    }

    #[test]
    fn closing_is_a_sequence_too_and_ends_on_the_page_before_the_dock() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::CloseWorkspace));
        let q = s.view().sequence.expect("the closing sequence");
        assert_eq!(q.title, "Closing shop");
        assert_eq!(q.phases.len(), 1);
        assert!(s.view().workspace.label == "shop");
        settle(&mut s);
        assert!(s.view().workspace.none);
        assert_eq!(s.route(), &Route::Welcome);
        assert!(s.view().toasts.iter().any(|t| t.text == "Closed shop"));
    }

    #[test]
    fn a_workspace_with_an_active_ticket_cannot_be_closed_until_it_is_parked() {
        use crate::sim::model::Activation;
        let mut sim = Sim::default();
        let i = sim.idx(&"PROJ-142".into()).unwrap();
        sim.tickets[i].stage = crate::sim::model::Stage::Active {
            held: Default::default(),
            act: Activation { started_ms: 0, records: vec![], overlay: None },
            prep: None,
        };
        let mut s = Session::new(Box::new(sim));
        s.handle(Intent::Do(Command::CloseWorkspace));
        settle(&mut s);
        let q = s.view().sequence.expect("stopped at the first step");
        assert_eq!(q.state, SequenceState::NeedsDecision);
        let steps = &q.phases[0].steps;
        assert_eq!(steps[0].state, ProgressState::Failed);
        assert!(steps[0].detail.contains("PROJ-142 is active. Park it first."), "{}", steps[0].detail);
        assert!(steps[1..].iter().all(|st| st.state == ProgressState::Skipped), "nothing after it is run");
        let labels: Vec<&str> = q.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Back", "Try again"], "no closing anyway");
        s.handle(Intent::Do(Command::ContinueSequence));
        assert!(s.view().sequence.is_some(), "continue is refused");
        s.handle(Intent::Do(Command::AbortSequence));
        assert_eq!(s.view().workspace.label, "shop", "still open");
    }

    #[test]
    fn the_landing_lists_five_and_the_rest_are_a_search_away() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        let ScreenVm::Welcome(w) = s.view().screen else {
            panic!()
        };
        assert_eq!(w.workspaces.len(), 6, "one more than the landing lists");
        // One line each: the name and its projects, not a folder.
        let shop = w.workspaces.iter().find(|i| i.name == "shop").unwrap();
        assert_eq!(shop.projects, ["api-client", "web", "worker", "docs"]);

        s.handle(Intent::OpenAllWorkspaces);
        let SheetVm::AllWorkspaces(a) = s.view().sheet.expect("the modal") else {
            panic!()
        };
        assert_eq!((a.items.len(), a.total), (6, 6));
        s.handle(Intent::SetText(Field::WorkspaceSearch, "HBT-web".into()));
        let SheetVm::AllWorkspaces(a) = s.view().sheet.unwrap() else {
            panic!()
        };
        assert_eq!(a.items.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["hbt"], "found by a project");
        s.handle(Intent::SetText(Field::WorkspaceSearch, "zzz".into()));
        let SheetVm::AllWorkspaces(a) = s.view().sheet.unwrap() else {
            panic!()
        };
        assert!(a.items.is_empty() && a.total == 6);

        // Picking one from the modal starts opening it.
        s.handle(Intent::SetText(Field::WorkspaceSearch, "pair".into()));
        s.handle(Intent::Do(Command::SelectWorkspace("pair".into())));
        assert_eq!(s.view().sequence.unwrap().title, "Opening pair");
    }

    #[test]
    fn the_branch_picker_says_what_switching_would_do_then_switches() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Workspace));
        s.handle(Intent::OpenBranchPicker);
        let SheetVm::Branches(b) = s.view().sheet.expect("the modal") else {
            panic!("not the branch picker")
        };
        assert!(!b.groups.is_empty(), "the workspace has branches to pick");
        assert_eq!(b.chosen, None);
        assert!(!b.switch.enabled, "nothing to switch to yet");

        s.handle(Intent::PickBranch(Branch::from("develop")));
        let SheetVm::Branches(b) = s.view().sheet.expect("still open") else {
            panic!("picking keeps the modal open")
        };
        assert_eq!(b.chosen.as_ref().map(Branch::as_str), Some("develop"));
        let effect = b.effect.expect("a choice shows what it would do");
        assert!(effect.summary.contains("develop"), "{}", effect.summary);

        // Confirming closes the modal and runs the switch as a sequence of steps.
        s.handle(Intent::Do(Command::SwitchBranch(Branch::from("develop"))));
        assert!(s.view().sheet.is_none());
        assert_eq!(s.view().sequence.as_ref().expect("the switch").title, "Switch to develop");
        settle(&mut s);
        match s.view().sequence {
            None => {}
            Some(q) => {
                // A project with uncommitted changes is refused and the decision is left to the
                // person; carrying on finishes the rest.
                assert_eq!(q.state, SequenceState::NeedsDecision);
                assert!(
                    q.phases.iter().flat_map(|p| &p.steps).any(|st| {
                        st.state == ProgressState::Failed
                            && st.detail.contains("uncommitted changes")
                    }),
                    "the refusal says why: {q:?}"
                );
                assert!(
                    q.actions.iter().any(|a| a.label == "Switch anyway"),
                    "carrying on is a switch: {q:?}"
                );
                s.handle(Intent::Do(Command::ContinueSequence));
                assert!(s.view().sequence.is_none(), "the switch finished");
            }
        }
    }

    #[test]
    fn the_palette_can_switch_workspace() {
        let mut s = Session::demo();
        s.handle(Intent::OpenPalette);
        let SheetVm::Palette { items, .. } = s.view().sheet.unwrap() else {
            panic!()
        };
        assert!(items.iter().any(|i| i.label == "Switch workspace…" && i.intent == Intent::ToggleWorkspacePicker));
        let open_hbt = items.iter().find(|i| i.label == "Open hbt").expect("a way to open hbt");
        s.handle(open_hbt.intent.clone());
        settle(&mut s);
        assert_eq!(s.view().workspace.label, "hbt");
    }

    #[test]
    fn the_page_before_the_dock_is_only_for_no_workspace_with_one_open_workspaces_is_the_menu() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Tickets(Group::All)));
        s.handle(Intent::Go(Route::Welcome));
        assert_eq!(s.route(), &Route::Tickets(Group::All), "the screen stays where it was");
        assert!(s.view().picker.is_some(), "and the workspace menu opens");
    }

    #[test]
    fn the_welcome_screen_shows_the_tools_the_workspaces_and_how_to_make_one() {
        let s = Session::new(Box::new(Sim::without_workspace()));
        let ScreenVm::Welcome(w) = s.view().screen else {
            panic!()
        };
        assert_eq!(w.providers.len(), 2, "Jira and GitHub");
        assert_eq!(w.init_command, "de init");
        assert_eq!(w.current, None);
        assert_eq!(w.workspaces.len(), 6);
        assert!(w.workspaces.iter().all(|i| !i.current));
        // `shop` was open until it was closed, so it was used last; `pair` was never opened.
        assert_eq!(w.workspaces.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["shop", "threadplay", "hbt", "quelle", "orbit", "pair"]);
        assert_eq!(w.workspaces[0].last_used, "just now");
        assert_eq!(w.workspaces[5].last_used, "never");
    }

    #[test]
    fn opening_a_workspace_from_the_list_goes_home_and_closing_goes_back_to_the_list() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        open_ws(&mut s, "hbt");
        assert_eq!(s.route(), &Route::Next);
        assert_eq!(ticket_keys(&s), ["HBT-204", "HBT-207"]);
        s.handle(Intent::Do(Command::CloseWorkspace));
        settle(&mut s);
        assert_eq!(s.route(), &Route::Welcome);
        assert!(s.view().workspace.none);
        assert!(ticket_keys(&s).is_empty());
        // Unknown names are refused with a way forward.
        open_ws(&mut s, "nope");
        assert!(s.view().toasts.iter().any(|t| t.text.contains("de init")), "{:?}", s.view().toasts);
    }

    #[test]
    fn the_workspace_menu_lists_the_open_one_then_the_others_and_narrows_by_search() {
        let mut s = Session::demo();
        assert!(s.view().picker.is_none());
        s.handle(Intent::ToggleWorkspacePicker);
        let p = s.view().picker.expect("open");
        assert_eq!(p.current.as_ref().map(|c| c.name.as_str()), Some("shop"));
        assert_eq!(p.recent.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["threadplay", "hbt", "quelle", "orbit", "pair"]);
        assert_eq!(p.total, 6);

        s.handle(Intent::SetText(Field::WorkspaceSearch, " TH ".into()));
        let p = s.view().picker.unwrap();
        assert_eq!(p.recent.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["threadplay"]);
        assert_eq!(p.current, None, "shop does not match");
        assert_eq!(p.total, 6, "the count is before narrowing");

        // Picking closes the menu; going anywhere closes it too.
        open_ws(&mut s, "threadplay");
        assert!(s.view().picker.is_none());
        s.handle(Intent::ToggleWorkspacePicker);
        s.handle(Intent::Go(Route::Settings));
        assert!(s.view().picker.is_none());
        s.handle(Intent::ToggleWorkspacePicker);
        s.handle(Intent::ToggleWorkspacePicker);
        assert!(s.view().picker.is_none(), "toggling twice closes it");
    }

    #[test]
    fn diagnose_shows_raw_output_and_reflects_sign_out() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::SetProvider {
            provider: Provider::GitHub,
            ready: false,
        }));
        s.handle(Intent::Diagnose);
        let SheetVm::Diagnose(text) = s.view().sheet.unwrap() else {
            panic!()
        };
        assert!(text.contains("not logged into any GitHub hosts"));
    }

    #[test]
    fn the_log_screen_opens_on_the_newest_run_and_switches_by_intent() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Logs));
        let ScreenVm::Logs(v) = s.view().screen else {
            panic!("not the log screen")
        };
        assert_eq!(v.selected.as_deref(), Some("sync-run-0.log"));
        assert!(v.lines[0].contains("sync-run-0.log"));
        assert!(v.runs.len() > 1 && !v.note.is_empty());

        s.handle(Intent::SelectLog("sync-run-2.log".into()));
        let ScreenVm::Logs(v) = s.view().screen else {
            panic!()
        };
        assert_eq!(v.selected.as_deref(), Some("sync-run-2.log"));
        assert!(v.lines[0].contains("sync-run-2.log"));
    }

    #[test]
    fn a_selected_log_that_was_pruned_is_not_shown() {
        let mut s = Session::demo();
        s.handle(Intent::SelectLog("sync-gone.log".into()));
        s.handle(Intent::Go(Route::Logs));
        let ScreenVm::Logs(v) = s.view().screen else {
            panic!()
        };
        assert_eq!(v.selected, None);
        assert!(v.lines.is_empty());
    }

    #[test]
    fn the_log_screen_without_a_workspace_says_that_runs_belong_to_one() {
        let mut s = Session::new(Box::new(Sim::without_workspace()));
        s.handle(Intent::Go(Route::Logs));
        let ScreenVm::Logs(v) = s.view().screen else {
            panic!("not the log screen")
        };
        assert!(v.runs.is_empty(), "runs belong to a workspace: {v:?}");
        assert_eq!(v.empty.title, "No workspace is open");
        assert_eq!(v.empty.actions.len(), 1, "the way to any run is to open one");

        // With a workspace open the same screen offers the way to the first run.
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Logs));
        let ScreenVm::Logs(v) = s.view().screen else {
            panic!("not the log screen")
        };
        assert_eq!(v.empty.title, "No sync logs yet");
    }

    #[test]
    fn screens_that_are_the_record_have_no_side_panel_echoing_it() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Audit));
        assert!(s.view().right.is_empty());
        s.handle(Intent::Go(Route::Logs));
        assert!(s.view().right.is_empty());
        s.handle(Intent::Go(Route::Settings));
        assert!(!s.view().right.is_empty());
    }

    fn mapping_value(s: &Session, key: MappingKey) -> String {
        s.store()
            .settings()
            .mapping
            .into_iter()
            .find(|r| r.key == key)
            .unwrap()
            .value
    }

    #[test]
    fn a_mapping_field_starts_from_the_stored_value_and_saves_when_submitted() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Settings));
        let f = Field::Mapping(MappingKey::AccountId);
        assert_eq!(s.text(&f), "acct-0001", "seeded from the store");

        s.handle(Intent::SetText(f.clone(), "  acct-9  ".into()));
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "acct-0001", "typing alone saves nothing");
        s.handle(Intent::Submit(f.clone()));
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "acct-9");
        assert!(s.view().toasts.iter().any(|t| t.text.contains("Saved")));

        // Clearing it puts the default back.
        s.handle(Intent::SetText(f.clone(), String::new()));
        s.handle(Intent::Submit(f));
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "");
    }

    #[test]
    fn an_unset_status_shows_its_default_as_text_and_typing_it_back_unsets_it() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Settings));
        let f = Field::Mapping(MappingKey::ReviewStatus);
        assert_eq!(s.text(&f), "In Review");
        assert_eq!(mapping_value(&s, MappingKey::ReviewStatus), "");

        // Submitting the untouched default changes nothing and says nothing.
        let before = s.view().toasts.len();
        s.handle(Intent::Submit(f.clone()));
        assert_eq!(s.view().toasts.len(), before);

        s.handle(Intent::SetText(f.clone(), "Code Review".into()));
        s.handle(Intent::Submit(f.clone()));
        assert_eq!(mapping_value(&s, MappingKey::ReviewStatus), "Code Review");

        s.handle(Intent::SetText(f.clone(), "In Review".into()));
        s.handle(Intent::Submit(f));
        assert_eq!(mapping_value(&s, MappingKey::ReviewStatus), "");
    }

    #[test]
    fn edits_are_saved_together_marked_until_then_and_kept_when_leaving_the_page() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Settings));
        let edited = |s: &Session| {
            let ScreenVm::Settings(v) = s.view().screen else {
                panic!()
            };
            v.edited
        };
        assert!(edited(&s).is_empty());

        s.handle(Intent::SetText(Field::Mapping(MappingKey::UatStatus), "Acceptance".into()));
        s.handle(Intent::SetText(Field::Mapping(MappingKey::AccountId), "acct-7".into()));
        assert_eq!(edited(&s), [MappingKey::UatStatus, MappingKey::AccountId]);
        assert_eq!(mapping_value(&s, MappingKey::UatStatus), "", "nothing is saved while typing");

        // Leaving the page and coming back keeps the edits.
        s.handle(Intent::Go(Route::Next));
        s.handle(Intent::Go(Route::Settings));
        assert_eq!(edited(&s).len(), 2);

        let toasts = s.view().toasts.len();
        s.handle(Intent::SaveMapping);
        assert_eq!(mapping_value(&s, MappingKey::UatStatus), "Acceptance");
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "acct-7");
        assert!(edited(&s).is_empty());
        assert_eq!(s.view().toasts.len(), toasts + 1, "one toast for the whole save");
    }

    #[test]
    fn discard_puts_the_fields_back_to_what_is_stored() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Settings));
        let f = Field::Mapping(MappingKey::AccountId);
        s.handle(Intent::SetText(f.clone(), "typo".into()));
        s.handle(Intent::DiscardMapping);
        assert_eq!(s.text(&f), "acct-0001");
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "acct-0001");
    }

    #[test]
    fn reset_asks_first_then_clears_everything_and_the_fields() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Settings));
        s.handle(Intent::Do(Command::ResetMapping));
        assert!(s.has_sheet(), "a reset is confirmed first");
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "acct-0001");

        s.handle(Intent::ConfirmSheet);
        assert_eq!(mapping_value(&s, MappingKey::AccountId), "");
        assert_eq!(mapping_value(&s, MappingKey::ReviewJql), "");
        assert_eq!(s.text(&Field::Mapping(MappingKey::AccountId)), "");
        assert_eq!(s.text(&Field::Mapping(MappingKey::ReviewStatus)), "In Review");
    }

    #[test]
    fn submitting_an_unchanged_mapping_field_says_nothing() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Settings));
        let before = s.view().toasts.len();
        s.handle(Intent::Submit(Field::Mapping(MappingKey::ReviewJql)));
        assert_eq!(s.view().toasts.len(), before);
    }

    #[test]
    fn every_remote_command_has_a_literal_preview() {
        let s = Session::demo();
        let key = k("PROJ-127");
        let cases = [
            Command::PostComment {
                key: key.clone(),
                text: "hi".into(),
            },
            Command::Approve {
                key: key.clone(),
                repo: "web".into(),
                pr: PrNumber(469),
            },
            Command::ApproveAll(key.clone()),
            Command::Rerun {
                key: key.clone(),
                repo: "web".into(),
            },
            Command::Transition(key.clone()),
            Command::RequestChanges {
                key: key.clone(),
                pr: PrNumber(469),
                text: "x".into(),
            },
            Command::InlineComment {
                key: key.clone(),
                pr: PrNumber(469),
                file: "src/session.rs".into(),
                line: LineAnchor::New(31),
                text: "x".into(),
            },
        ];
        for c in cases {
            let Classified::Remote(r) = c.clone().classify() else {
                panic!("{c:?} should be remote")
            };
            let p = s
                .store()
                .preview(&r)
                .unwrap_or_else(|e| panic!("{c:?}: {e}"));
            assert!(!p.payload.is_empty(), "{c:?} shows its literal payload");
            assert_eq!(p.command(), &c);
        }
        // a vanished ticket is an error, never a silent preview
        let Classified::Remote(r) = Command::ApproveAll(k("PROJ-999")).classify() else {
            panic!()
        };
        assert!(s.store().preview(&r).is_err());
    }

    #[test]
    fn empty_lists_say_what_belongs_there_and_what_to_do() {
        let mut s = Session::demo();
        // a list that is empty in the demo
        s.handle(Intent::go_tickets(Group::Done));
        let ScreenVm::Tickets(v) = s.view().screen else {
            panic!("not a ticket list")
        };
        assert!(v.sections.is_empty());
        assert!(!v.empty.hint.is_empty());
        assert!(
            v.empty
                .actions
                .iter()
                .any(|a| matches!(a.intent, Intent::Go(_))),
            "it points at where the tickets are"
        );
        // every group has a real explanation, never a bare "Nothing"
        for g in Group::LIST {
            let e = s.store().tickets(g).empty;
            assert!(e.hint.len() > 20, "{g:?}: {e:?}");
        }
        // rows exist but the search hides them: the way back is offered
        s.handle(Intent::go_tickets(Group::All));
        s.handle(Intent::SetText(Field::TicketFilter, "zzz-no-such".into()));
        let ScreenVm::Tickets(v) = s.view().screen else {
            panic!()
        };
        assert_eq!(v.empty.title, "No ticket matches");
        s.handle(v.empty.actions[0].intent.clone());
        assert_eq!(s.text(&Field::TicketFilter), "");
    }

    #[test]
    fn the_ticket_table_searches_filters_and_sorts() {
        let mut s = Session::demo();
        s.handle(Intent::go_tickets(Group::All));
        let list = |s: &Session| match s.view().screen {
            ScreenVm::Tickets(v) => v,
            _ => panic!("not the ticket list"),
        };
        let keys = |s: &Session| -> Vec<String> {
            list(s)
                .sections
                .iter()
                .flat_map(|x| x.rows.iter().map(|r| r.key.to_string()))
                .collect()
        };
        let all = keys(&s);
        assert!(all.len() >= 5);
        // search
        s.handle(Intent::SetText(Field::TicketFilter, "zzz-no-such".into()));
        assert!(keys(&s).is_empty());
        assert!(
            list(&s).total > 0,
            "an empty result is told apart from an empty group"
        );
        s.handle(Intent::SetText(Field::TicketFilter, String::new()));
        // filters combine: a repo and hotfix
        s.handle(Intent::ToggleTicketFilter(TicketFilter::Hotfix));
        let hot = keys(&s);
        assert!(!hot.is_empty() && hot.len() < all.len());
        s.handle(Intent::ClearTicketFilters);
        assert_eq!(keys(&s), all);
        // sort: ascending, descending, then back to the default order
        s.handle(Intent::SortTickets(TicketSort::Key));
        let mut asc = all.clone();
        asc.sort();
        assert_eq!(keys(&s), asc);
        s.handle(Intent::SortTickets(TicketSort::Key));
        asc.reverse();
        assert_eq!(keys(&s), asc);
        s.handle(Intent::SortTickets(TicketSort::Key));
        assert_eq!(keys(&s), all);
    }

    #[test]
    fn the_sidebar_lists_the_statuses_under_their_own_title() {
        let s = Session::demo();
        let nav = s.view().nav;
        let labels =
            |i: usize| -> Vec<&str> { nav[i].items.iter().map(|x| x.label.as_str()).collect() };
        assert_eq!(labels(0), ["Home", "Review pool", "All tickets"]);
        assert_eq!(nav[1].title, "Status");
        assert_eq!(
            labels(1),
            [
                "In review",
                "Active",
                "Parked",
                "Awaiting alpha",
                "Returned",
                "Done"
            ]
        );
    }

    #[test]
    fn on_uat_shares_the_ticket_table_and_each_list_starts_fresh() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::OnUat));
        let rows = |s: &Session| match s.view().screen {
            ScreenVm::OnUat(v) => v.table.sections.iter().map(|x| x.rows.len()).sum::<usize>(),
            _ => panic!("not the uat screen"),
        };
        let all = rows(&s);
        assert!(all >= 2);
        s.handle(Intent::ToggleTicketFilter(TicketFilter::Repo(
            RepoName::from("worker"),
        )));
        assert_eq!(rows(&s), 0, "nothing on uat touches worker");
        s.handle(Intent::SetText(Field::TicketFilter, "x".into()));
        // moving to another list clears what was set here
        s.handle(Intent::go_tickets(Group::All));
        assert_eq!(s.text(&Field::TicketFilter), "");
        s.handle(Intent::Go(Route::OnUat));
        assert_eq!(rows(&s), all);
    }

    #[test]
    fn the_next_filter_narrows_the_list_by_ticket_title_or_reason() {
        let mut s = Session::demo();
        let count = |s: &Session| match s.view().screen {
            ScreenVm::Next(n) => n.cards.len(),
            _ => panic!("not on Next"),
        };
        let all = count(&s);
        s.handle(Intent::SetText(Field::NextFilter, "proj-139".into()));
        let some = count(&s);
        assert!(some > 0 && some < all, "{some} of {all}");
        s.handle(Intent::SetText(Field::NextFilter, "zzz-no-such".into()));
        assert_eq!(count(&s), 0);
        s.handle(Intent::SetText(Field::NextFilter, String::new()));
        assert_eq!(count(&s), all);
    }

    #[test]
    fn snooze_choices_say_their_unit() {
        let s = Session::demo();
        let card = &s.store().next(false).cards[0];
        let labels: Vec<&str> = card.snooze.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["2 minutes", "10 minutes"]);
    }
}

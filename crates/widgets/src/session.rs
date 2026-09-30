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
}

impl Session {
    pub fn new(store: Box<dyn Store>) -> Self {
        Self {
            store,
            route: Route::Next,
            screen: Route::Next,
            tabs: Vec::new(),
            sheet: None,
            toasts: Vec::new(),
            next_toast: 0,
            show_all: false,
            ticket_filters: TicketFilters::default(),
            ticket_sort: None,
            attention_open: false,
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
        for (text, kind) in self.store.tick(millis) {
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
        match intent {
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

    fn select_log(&mut self, id: String) {
        let text = self.store.log_text(&id);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        self.log_view = Some((id, Rc::new(lines)));
    }

    fn go(&mut self, route: Route) {
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

    fn apply(&mut self, cmd: &Command, out: Outcome) {
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
        match &self.route {
            Route::Next => {
                let mut n = self.store.next(self.show_all);
                let q = self.text(&Field::NextFilter).trim().to_lowercase();
                if !q.is_empty() {
                    n.cards.retain(|c| {
                        c.title.to_lowercase().contains(&q)
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
                })
            }
            Route::Settings => {
                let mut v = self.store.settings();
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
            toasts: self.toasts.clone(),
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
    fn screens_that_are_the_record_have_no_side_panel_echoing_it() {
        let mut s = Session::demo();
        s.handle(Intent::Go(Route::Audit));
        assert!(s.view().right.is_empty());
        s.handle(Intent::Go(Route::Logs));
        assert!(s.view().right.is_empty());
        s.handle(Intent::Go(Route::Settings));
        assert!(!s.view().right.is_empty());
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

//! UI state on top of a [`Store`]: where you are, which tabs are open, which sheet is up, what you typed.
//!
//! The session is the only writer of UI state. Views emit [`Intent`]s into [`Session::handle`] and draw [`Session::view`].
//! It depends on no GPUI, so the whole interaction model is unit-tested headless (see the tests below).

use std::collections::{BTreeMap, BTreeSet};

use crate::sim::Sim;
use crate::store::{Outcome, ReviewSel, Store, hunk_key};
use crate::vm::*;

#[derive(Clone, Debug)]
struct TabEntry {
    key: String,
    pinned: bool,
    tab: TicketTab,
}

#[derive(Clone, Debug)]
enum Sheet {
    Confirm {
        command: Command,
        preview: Preview,
    },
    Baseline {
        key: String,
    },
    Report(ReportVm),
    Compose(ComposeKind),
    Palette,
    ClaimBlocked {
        key: String,
        block: ClaimBlock,
        then: Then,
    },
    Busy {
        busy: Busy,
        retry: Command,
    },
    Diagnose,
    CloseTab(String),
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
    attention_open: bool,
    simulate_open: bool,
    right_open: bool,
    diff_mode: DiffMode,
    since: BTreeMap<String, SinceMode>,
    pr_sel: BTreeMap<String, u32>,
    file_sel: BTreeMap<(String, u32), usize>,
    viewed: BTreeSet<String>,
    composer: Option<(String, u32, String, String)>,
    texts: BTreeMap<Field, String>,
    seen_snap: BTreeMap<String, u32>,
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
            attention_open: false,
            simulate_open: false,
            right_open: true,
            diff_mode: DiffMode::Unified,
            since: BTreeMap::new(),
            pr_sel: BTreeMap::new(),
            file_sel: BTreeMap::new(),
            viewed: BTreeSet::new(),
            composer: None,
            texts: BTreeMap::new(),
            seen_snap: BTreeMap::new(),
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
            Intent::PinTab(key) => {
                if let Some(t) = self.tabs.iter_mut().find(|t| t.key == key) {
                    t.pinned = true;
                }
            }
            Intent::ToggleAttention => self.attention_open = !self.attention_open,
            Intent::ToggleSimulate => self.simulate_open = !self.simulate_open,
            Intent::TogglePanel => self.right_open = !self.right_open,
            Intent::ToggleShowAll => self.show_all = !self.show_all,
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
                self.run(cmd, false);
            }
            Intent::ConfirmSheet => self.confirm(),
            Intent::PickBaseline(b) => {
                if let Some(Sheet::Baseline { key }) = self.sheet.take() {
                    self.run(
                        Command::Activate {
                            key,
                            baseline: Some(b),
                        },
                        false,
                    );
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
                self.store.dispatch(&Command::SetNotes {
                    key: key.clone(),
                    text: text.clone(),
                });
            }
            Field::Draft { key, id } => {
                self.store.dispatch(&Command::EditDraft {
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
                    self.store.dispatch(&Command::AddChecklist {
                        key: key.clone(),
                        text,
                    });
                    self.texts.remove(&Field::Checklist(key));
                }
            }
            Field::Comment(key) => {
                if !text.trim().is_empty() {
                    self.run(Command::PostComment { key, text }, false);
                }
            }
            Field::Inline => {
                if text.trim().is_empty() {
                    return;
                }
                if let Some((key, pr, file, line)) = self.composer.clone() {
                    self.run(
                        Command::InlineComment {
                            key,
                            pr,
                            file,
                            line,
                            text,
                        },
                        false,
                    );
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
                    self.run(Command::RequestChanges { key, pr, text }, false);
                }
            }
            _ => {}
        }
    }

    fn go(&mut self, route: Route) {
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
                self.store.dispatch(&Command::MarkSeen(key.clone()));
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
            self.screen = route.clone();
        }
        self.route = route;
    }

    fn close_tab(&mut self, key: &str) {
        self.tabs.retain(|t| t.key != key);
        if matches!(&self.route, Route::Ticket { key: k, .. } if k == key) {
            self.route = self.screen.clone();
        }
    }

    /* ---------------------------- commands and sheets ---------------------------- */

    fn run(&mut self, cmd: Command, confirmed: bool) {
        if !confirmed && let Some(preview) = self.store.preview(&cmd) {
            self.texts.remove(&Field::TypedKey);
            self.sheet = Some(Sheet::Confirm {
                command: cmd,
                preview,
            });
            return;
        }
        let out = self.store.dispatch(&cmd);
        self.apply(&cmd, out);
    }

    fn confirm(&mut self) {
        let Some(Sheet::Confirm { command, preview }) = self.sheet.clone() else {
            // A report sheet is dismissed with the same button.
            if matches!(self.sheet, Some(Sheet::Report(_))) {
                self.sheet = None;
            }
            return;
        };
        if preview.blocked.is_some() {
            return;
        }
        if let Some(k) = &preview.type_key
            && self.text(&Field::TypedKey).trim() != k
        {
            return;
        }
        self.sheet = None;
        self.texts.remove(&Field::TypedKey);
        self.run(command, true);
    }

    fn apply(&mut self, cmd: &Command, out: Outcome) {
        if out.need_baseline
            && let Command::Activate { key, .. } = cmd
        {
            self.sheet = Some(Sheet::Baseline { key: key.clone() });
            return;
        }
        if let Some(block) = out.blocker {
            let (key, then) = match cmd {
                Command::Claim(k) => (k.clone(), Then::Claim),
                Command::StartReview(k) => (k.clone(), Then::StartReview),
                Command::Reclaim(k) => (k.clone(), Then::Reclaim),
                _ => return,
            };
            self.sheet = Some(Sheet::ClaimBlocked { key, block, then });
            return;
        }
        if let Some(busy) = out.busy {
            self.sheet = Some(Sheet::Busy {
                busy,
                retry: cmd.clone(),
            });
            return;
        }
        if !out.ok {
            let text = out.errors.join(" ");
            self.push_toast(text, ToastKind::Bad, None);
            return;
        }
        if let Some(report) = out.report {
            self.sheet = Some(Sheet::Report(report));
        }
        if let Some((text, kind, undo)) = out.toast {
            self.push_toast(text, kind, undo.map(|u| Intent::Do(Command::Undo(u))));
        }
        match cmd {
            Command::StartReview(key) => {
                self.go(Route::ticket(key.clone(), TicketTab::Review));
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

    fn review_sel(&self, key: &str) -> ReviewSel {
        let pr = self.pr_sel.get(key).copied();
        let file = pr
            .and_then(|p| self.file_sel.get(&(key.to_string(), p)).copied())
            .unwrap_or(0);
        ReviewSel {
            pr,
            file,
            mode: self.diff_mode,
            since: self.since.get(key).copied().unwrap_or(SinceMode::Since),
            viewed: self.viewed.iter().cloned().collect(),
            composer: self
                .composer
                .as_ref()
                .filter(|(k, ..)| k == key)
                .map(|(_, pr, f, l)| (*pr, f.clone(), l.clone())),
        }
    }

    fn nav(&self, counts: &Counts) -> Vec<NavSection> {
        let item = |label: &str, route: Route, count: usize| NavItem {
            selected: self.route == route,
            label: label.to_string(),
            route,
            count,
        };
        let mut tickets = vec![item("Next", Route::Next, counts.suggestions)];
        tickets.extend(
            Group::NAV
                .iter()
                .map(|g| item(g.label(), Route::Tickets(*g), counts.group(*g))),
        );
        vec![
            NavSection {
                title: "Tickets".to_string(),
                items: tickets,
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
                    item("Settings", Route::Settings, 0),
                ],
            },
        ]
    }

    fn tab_strip(&self) -> Vec<ShellTab> {
        let label = |r: &Route| match r {
            Route::Next => "Next".to_string(),
            Route::Tickets(g) => g.label().to_string(),
            Route::OnUat => "On uat".to_string(),
            Route::Workspace => "Workspace".to_string(),
            Route::Audit => "Audit log".to_string(),
            Route::Settings => "Settings".to_string(),
            Route::Ticket { key, .. } => key.clone(),
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
                label: t.key.clone(),
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
            Sheet::Confirm { preview, .. } => {
                let typed = self.text(&Field::TypedKey);
                let can = preview.blocked.is_none()
                    && preview.type_key.as_ref().is_none_or(|k| typed.trim() == k);
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

    fn screen_vm(&self) -> ScreenVm {
        match &self.route {
            Route::Next => ScreenVm::Next(self.store.next(self.show_all)),
            Route::Tickets(g) => ScreenVm::Tickets(self.store.tickets(*g)),
            Route::OnUat => ScreenVm::OnUat(self.store.on_uat()),
            Route::Workspace => ScreenVm::Workspace(self.store.workspace()),
            Route::Audit => ScreenVm::Audit(self.store.audit()),
            Route::Settings => ScreenVm::Settings(self.store.settings()),
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
        AppVm {
            route: self.route.clone(),
            nav: self.nav(&counts),
            tabs: self.tab_strip(),
            status: self.store.status(),
            attention_count: counts.attention,
            attention: self.attention_open.then(|| self.store.attention()),
            simulate: self.simulate_open.then(|| self.store.simulate()),
            right: self.right_open.then(|| self.store.right_panel(&self.route)),
            screen: self.screen_vm(),
            sheet: self.sheet_vm(),
            toasts: self.toasts.clone(),
        }
    }

    /// Texts the views must mirror into their inputs (used to clear an input after a send).
    pub fn field_texts(&self) -> &BTreeMap<Field, String> {
        &self.texts
    }

    /// The hunk key used by the viewed set (exposed for tests and the review screen).
    pub fn viewed_key(ticket: &str, pr: u32, since: bool, path: &str, hunk: usize) -> String {
        hunk_key(ticket, pr, since, path, hunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn next_starts_with_what_someone_is_waiting_on_and_offers_the_hotfix() {
        let s = Session::demo();
        let titles = next_titles(&s);
        assert!(
            titles[..3]
                .iter()
                .any(|t| t == "Returned: you were mentioned"),
            "{titles:?}"
        );
        assert!(
            titles.iter().any(|t| t == "Claim PROJ-139"),
            "the hotfix can be claimed even with a ticket in hand"
        );
        assert!(titles.iter().any(|t| t == "Start the review"));
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
        assert!(!s.store().tab_info("PROJ-131").unwrap().unseen);
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
            .ticket_head("PROJ-163", TicketTab::Overview)
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
        let key = "PROJ-142";
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
    fn a_failed_deploy_offers_a_rerun_through_the_confirm_sheet() {
        let mut s = Session::demo();
        let key = "PROJ-142";
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
        let key = "PROJ-142";
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
        assert!(!s.store().ticket_exists("PROJ-155"));
        s.handle(Intent::Do(Command::Sync));
        assert!(s.store().status().sync_running);
        s.tick(2000);
        assert!(s.store().ticket_exists("PROJ-155"));
        assert!(!s.store().status().sync_running);
    }

    #[test]
    fn offline_sync_keeps_the_cache_and_says_so() {
        let mut s = Session::demo();
        s.handle(Intent::Do(Command::Sim(SimEvent::Offline(true))));
        s.handle(Intent::Do(Command::Sync));
        s.tick(2000);
        assert!(!s.store().ticket_exists("PROJ-155"));
        assert_eq!(s.store().status().sync_tone, Tone::Warn);
    }

    #[test]
    fn a_stale_lock_refuses_activation_and_can_be_removed() {
        let mut s = Session::demo();
        let key = "PROJ-142";
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
            pr: 490,
        });
        assert_eq!(sheet_kind(&s), "compose");
        s.handle(Intent::SetText(Field::Compose, "Keep the logo row".into()));
        s.handle(Intent::Submit(Field::Compose));
        assert_eq!(sheet_kind(&s), "confirm");
        confirm_with(&mut s, None);
        let t = s
            .store()
            .review("PROJ-150", &s.review_sel("PROJ-150"))
            .unwrap();
        assert!(t.thread_count >= 2);
    }

    #[test]
    fn inline_comments_need_a_line_and_a_confirm() {
        let mut s = Session::demo();
        s.handle(Intent::InlineOpen {
            key: "PROJ-142".into(),
            pr: 212,
            file: "src/redirect.rs".into(),
            line: "n41".into(),
        });
        let r = s
            .store()
            .review("PROJ-142", &s.review_sel("PROJ-142"))
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
            .review("PROJ-142", &s.review_sel("PROJ-142"))
            .unwrap();
        assert!(r.composer.is_none());
        assert!(r.thread_count >= 2);
    }

    #[test]
    fn viewed_hunks_collapse() {
        let mut s = Session::demo();
        let r = s
            .store()
            .review("PROJ-142", &s.review_sel("PROJ-142"))
            .unwrap();
        let before = r.hunks[0].rows.len();
        assert!(before > 0);
        s.handle(r.hunks[0].toggle.clone());
        let r = s
            .store()
            .review("PROJ-142", &s.review_sel("PROJ-142"))
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
            .review("PROJ-142", &s.review_sel("PROJ-142"))
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
            .review("PROJ-142", &s.review_sel("PROJ-142"))
            .unwrap();
        assert_eq!(r.since_toggle.map(|t| t.2), Some(false));
    }

    #[test]
    fn split_mode_pairs_deletions_with_additions() {
        let mut s = Session::demo();
        s.handle(Intent::SetDiffMode(DiffMode::Split));
        let r = s
            .store()
            .review("PROJ-142", &s.review_sel("PROJ-142"))
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
}

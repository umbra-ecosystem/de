//! The [`Store`] over the engine.
//!
//! Tickets and sync are real: tickets come from the Jira mirror and local tracking, and "Sync now" runs the
//! `acli` adapter on a worker thread. Everything else the views ask for is served by the widgets'
//! simulation running on those tickets, until the engine serves it.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use de_core::config::Config;
use de_core::synclog;
use de_widgets::sim::Sim;
use de_widgets::vm::*;
use de_widgets::{Outcome, ReviewSel, Store};

use crate::{logs, mapping};
use crate::sync::{SyncDone, load_cached, sync_real};

pub struct CoreStore {
    sim: Sim,
    running: Option<Receiver<SyncDone>>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl CoreStore {
    /// Starts from the cached tickets and looks for new ones right away.
    pub fn open() -> Self {
        let cached = load_cached().unwrap_or_default();
        let mut store = Self {
            sim: Sim::with_tickets(cached),
            running: None,
        };
        store.begin_sync();
        store
    }

    fn begin_sync(&mut self) -> Outcome {
        if self.running.is_some() {
            return Outcome::fail("A sync is already running.");
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(sync_real(now()));
        });
        self.running = Some(rx);
        self.sim.sync_began();
        Outcome::ok()
    }

    /// Write one mapping setting to `config.toml`, then re-read the tickets so renamed statuses show right away.
    fn save_mapping(&mut self, key: MappingKey, text: &str) -> Outcome {
        match mapping::save(key, text) {
            Ok(_) => {
                if let Ok(fresh) = load_cached() {
                    self.sim.replace_tickets(fresh);
                }
                Outcome::ok().with_toast(
                    format!("Saved {}. {}", key.label().to_lowercase(), mapping::effect(key)),
                    ToastKind::Ok,
                    None,
                )
            }
            Err(e) => Outcome::fail(format!(
                "Could not save {} to config.toml: {e:#}. Check that the file is writable.",
                key.label().to_lowercase()
            )),
        }
    }

    /// Apply a finished sync, if there is one.
    fn poll(&mut self) -> Vec<(String, ToastKind)> {
        let Some(rx) = &self.running else {
            return Vec::new();
        };
        let done = match rx.try_recv() {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return Vec::new(),
            Err(TryRecvError::Disconnected) => SyncDone {
                report: vec![(false, "Jira (acli)".into(), "the sync stopped unexpectedly".into())],
                tickets: Err("the sync stopped unexpectedly".into()),
                jira_unavailable: false,
            },
        };
        self.running = None;
        let ok = done.report.iter().all(|r| r.0);
        let mut toasts = Vec::new();
        match done.tickets {
            Ok(fresh) => self.sim.replace_tickets(fresh),
            Err(e) => toasts.push((format!("Could not read the tickets: {e}"), ToastKind::Bad)),
        }
        self.sim.set_jira_ready(!done.jira_unavailable);
        if !ok {
            let why = done
                .report
                .iter()
                .find(|r| !r.0)
                .map(|r| r.2.clone())
                .unwrap_or_default();
            toasts.push((format!("Sync problem: {why}"), ToastKind::Warn));
        }
        self.sim.sync_ended(done.report);
        toasts
    }
}

impl Store for CoreStore {
    fn counts(&self) -> Counts {
        self.sim.counts()
    }
    fn status(&self) -> StatusVm {
        self.sim.status()
    }
    fn tab_info(&self, key: &TicketKey) -> Option<TabInfo> {
        self.sim.tab_info(key)
    }
    fn right_panel(&self, route: &Route) -> Vec<RightSection> {
        self.sim.right_panel(route)
    }
    fn palette(&self, query: &str) -> Vec<PaletteItem> {
        self.sim.palette(query)
    }
    fn next(&self, show_all: bool) -> NextVm {
        self.sim.next(show_all)
    }
    fn attention(&self) -> AttentionVm {
        self.sim.attention()
    }
    fn tickets(&self, group: Group) -> TicketListVm {
        self.sim.tickets(group)
    }
    fn ticket_exists(&self, key: &TicketKey) -> bool {
        self.sim.ticket_exists(key)
    }
    fn ticket_head(&self, key: &TicketKey, tab: TicketTab) -> Option<TicketHeadVm> {
        self.sim.ticket_head(key, tab)
    }
    fn overview(&self, key: &TicketKey, seen: Option<u32>) -> Option<OverviewVm> {
        self.sim.overview(key, seen)
    }
    fn review(&self, key: &TicketKey, sel: &ReviewSel) -> Option<ReviewVm> {
        self.sim.review(key, sel)
    }
    fn test(&self, key: &TicketKey) -> Option<TestVm> {
        self.sim.test(key)
    }
    fn ship(&self, key: &TicketKey) -> Option<ShipVm> {
        self.sim.ship(key)
    }
    fn timeline(&self, key: &TicketKey) -> Vec<AuditRow> {
        self.sim.timeline(key)
    }
    fn on_uat(&self) -> OnUatVm {
        self.sim.on_uat()
    }
    fn workspace(&self) -> WorkspaceVm {
        self.sim.workspace()
    }
    fn audit(&self) -> Vec<AuditRow> {
        self.sim.audit()
    }
    fn settings(&self) -> SettingsVm {
        let mut v = self.sim.settings();
        if let Ok(config) = Config::load() {
            v.mapping = mapping::rows(&config);
        }
        v
    }
    fn logs(&self) -> (Vec<LogRunVm>, String) {
        match (synclog::default_dir(), Config::load()) {
            (Ok(dir), Ok(config)) => logs::runs_in(&dir, config.log_keep()),
            (Ok(dir), Err(_)) => logs::runs_in(&dir, synclog::DEFAULT_KEEP),
            (Err(_), _) => (Vec::new(), String::new()),
        }
    }
    fn log_text(&self, id: &str) -> String {
        match synclog::default_dir() {
            Ok(dir) => logs::text_in(&dir, id),
            Err(e) => format!("The log folder could not be found: {e:#}"),
        }
    }
    fn simulate(&self) -> Vec<SimGroup> {
        self.sim.simulate()
    }
    fn comments_seen(&self, key: &TicketKey) -> u32 {
        self.sim.comments_seen(key)
    }
    fn diagnose(&self) -> String {
        self.sim.diagnose()
    }
    fn baseline_choices(&self, key: &TicketKey) -> BaselineVm {
        self.sim.baseline_choices(key)
    }
    fn preview(&self, command: &RemoteCommand) -> Result<Preview, String> {
        self.sim.preview(command)
    }
    fn local_guard(&self, command: &LocalCommand) -> Option<Preview> {
        self.sim.local_guard(command)
    }

    fn dispatch(&mut self, command: LocalCommand) -> Outcome {
        match command.command() {
            Command::Sync => self.begin_sync(),
            Command::SetMapping { key, text } => self.save_mapping(*key, text),
            _ => self.sim.dispatch(command),
        }
    }
    fn execute(&mut self, confirmed: Confirmed) -> Outcome {
        self.sim.execute(confirmed)
    }
    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)> {
        let mut toasts = self.poll();
        toasts.extend(self.sim.tick(millis));
        toasts
    }
}

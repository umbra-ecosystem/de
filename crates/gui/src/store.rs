//! The [`Store`] over the engine.
//!
//! Tickets and sync are real: tickets come from the Jira mirror and local tracking, and "Sync now" runs the
//! `acli` adapter on a worker thread. Everything else the views ask for is served by the widgets'
//! simulation running on those tickets, until the engine serves it.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use de_core::config::Config;
use de_core::store::{Kind, Store as Db};
use de_core::synclog;
use de_widgets::sim::Sim;
use de_widgets::sim::model::MS_PER_MIN;
use de_widgets::vm::*;
use de_widgets::{Outcome, ReviewSel, Store};

use crate::sync::{SyncDone, last_sync_minutes_ago, load_cached, sync_real};
use crate::{audit, localtime, logs, mapping, schedule};

pub struct CoreStore {
    sim: Sim,
    running: Option<Receiver<SyncDone>>,
    /// Real milliseconds not yet turned into simulated ones (see [`CoreStore::tick`]).
    carry_ms: i64,
    /// Minutes between automatic syncs (0 is off), when the last sync was started (unix seconds), how many in a
    /// row failed, and whether the running one was automatic. See [`crate::schedule`].
    interval_minutes: u32,
    last_attempt: i64,
    failures: u32,
    running_auto: bool,
}

/// Real milliseconds (plus what was carried over) as simulated milliseconds, and what is left over. A real minute
/// is `MS_PER_MIN` simulated milliseconds.
fn to_sim_ms(carry: i64, real: i64) -> (i64, i64) {
    let per_sim_ms = 60_000 / MS_PER_MIN;
    let total = carry + real;
    (total / per_sim_ms, total % per_sim_ms)
}

/// The audit log from `state.db`, as the window's entries.
fn stored_audit() -> eyre::Result<Vec<de_widgets::sim::model::AuditEntry>> {
    audit::entries(&Db::open_default(Kind::State)?, now())
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
        let mut sim = Sim::with_tickets(cached);
        sim.set_last_sync_minutes_ago(last_sync_minutes_ago(now()));
        sim.set_start_minute_of_day(localtime::minute_of_day(now()));
        // Pull requests come from GitHub, which the app does not read yet: nothing is said about them.
        sim.set_github_ready(false);
        sim.set_wall_clock_start(now());
        // The audit log is on disk; the simulation only shows it.
        let stored = stored_audit().unwrap_or_default();
        // The side panel's "Last sync" is the last one on record, not only one from this session.
        sim.set_last_sync_report(audit::last_sync_report(&stored));
        sim.use_external_audit(stored);
        let interval_minutes = Config::load()
            .map(|c| c.sync_interval_minutes())
            .unwrap_or(de_core::config::DEFAULT_SYNC_INTERVAL_MINUTES);
        sim.set_sync_interval(
            interval_minutes,
            schedule::overdue_after_minutes(interval_minutes),
        );
        let mut store = Self {
            sim,
            running: None,
            carry_ms: 0,
            interval_minutes,
            last_attempt: 0,
            failures: 0,
            running_auto: false,
        };
        store.begin_sync();
        store
    }

    /// Write one line to the audit log and show it. A log that cannot be written is reported to the user, since
    /// an action nobody can see afterwards is a problem of its own.
    fn record(&mut self, action: &str, ok: bool, text: &str) -> Option<(String, ToastKind)> {
        let saved = Db::open_default(Kind::State)
            .and_then(|state| audit::record(&state, now(), action, None, ok, text).map(|()| state))
            .and_then(|state| audit::entries(&state, now()));
        match saved {
            Ok(entries) => {
                self.sim.set_audit(entries);
                None
            }
            Err(e) => Some((
                format!(
                    "Could not write to the audit log: {e:#}. Check that the data folder is writable."
                ),
                ToastKind::Warn,
            )),
        }
    }

    fn begin_sync(&mut self) -> Outcome {
        self.start_sync(false)
    }

    /// Start a sync on a worker thread. A manual one (`auto` false) also restarts the automatic timer.
    fn start_sync(&mut self, auto: bool) -> Outcome {
        if self.running.is_some() {
            return Outcome::fail("A sync is already running.");
        }
        self.last_attempt = now();
        self.running_auto = auto;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(sync_real(now()));
        });
        self.running = Some(rx);
        self.sim.sync_began();
        Outcome::ok()
    }

    /// Write mapping settings to `config.toml` (or clear them all), then re-read the tickets so renamed statuses
    /// show right away. `None` when `command` is not a mapping command.
    fn mapping_command(&mut self, command: &Command) -> Option<Outcome> {
        let (saved, done) = match command {
            Command::SaveMapping { changes } => (
                mapping::save_all(changes),
                format!(
                    "Saved {} setting{}. {}",
                    changes.len(),
                    if changes.len() == 1 { "" } else { "s" },
                    mapping::effect(changes.iter().map(|(k, _)| *k))
                ),
            ),
            Command::ResetMapping => (
                mapping::reset(),
                "Jira mapping reset to defaults. Status names apply now; the review query from the next sync."
                    .to_string(),
            ),
            _ => return None,
        };
        Some(match saved {
            Ok(_) => {
                if let Ok(fresh) = load_cached() {
                    self.sim.replace_tickets(fresh);
                }
                match self.record("config.jira", true, &done) {
                    Some((warning, kind)) => Outcome::ok().with_toast(warning, kind, None),
                    None => Outcome::ok().with_toast(done, ToastKind::Ok, None),
                }
            }
            Err(e) => {
                let why = format!("Could not save to config.toml: {e:#}");
                self.record("config.jira", false, &why);
                Outcome::fail(format!(
                    "{why}. Check that the file is writable, then try again."
                ))
            }
        })
    }

    /// Change how often the app syncs by itself, in `config.toml`; 0 turns it off.
    fn set_sync_interval(&mut self, minutes: u32) -> Outcome {
        let saved = Config::mutate_persisted(|config| {
            let sync = config.sync.get_or_insert_with(Default::default);
            sync.interval_minutes = Some(minutes);
        });
        match saved {
            Ok(_) => {
                self.interval_minutes = minutes;
                self.failures = 0;
                self.sim
                    .set_sync_interval(minutes, schedule::overdue_after_minutes(minutes));
                let said = if minutes == 0 {
                    "Automatic sync is off".to_string()
                } else {
                    format!("Syncing every {minutes} minutes")
                };
                self.record("config.sync", true, &said);
                Outcome::ok().with_toast(said, ToastKind::Ok, None)
            }
            Err(e) => {
                let why = format!("Could not save to config.toml: {e:#}");
                self.record("config.sync", false, &why);
                Outcome::fail(format!(
                    "{why}. Check that the file is writable, then try again."
                ))
            }
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
                report: vec![(
                    false,
                    "Jira (acli)".into(),
                    "the sync stopped unexpectedly".into(),
                )],
                tickets: Err("the sync stopped unexpectedly".into()),
                jira_unavailable: false,
                changed: true,
            },
        };
        self.running = None;
        let auto = std::mem::take(&mut self.running_auto);
        let ok = done.report.iter().all(|r| r.0);
        let mut toasts = Vec::new();
        match done.tickets {
            Ok(fresh) => self.sim.replace_tickets(fresh),
            Err(e) => toasts.push((format!("Could not read the tickets: {e}"), ToastKind::Bad)),
        }
        self.sim.set_jira_ready(!done.jira_unavailable);
        self.failures = if ok { 0 } else { self.failures + 1 };
        // An automatic sync is quiet: a problem is said once, when it starts, not every cycle.
        if !ok && (!auto || self.failures == 1) {
            let why = done
                .report
                .iter()
                .find(|r| !r.0)
                .map(|r| r.2.clone())
                .unwrap_or_default();
            toasts.push((format!("Sync problem: {why}"), ToastKind::Warn));
        }
        let text = done
            .report
            .iter()
            .map(|r| r.2.clone())
            .collect::<Vec<_>>()
            .join("; ");
        self.sim.sync_ended(done.report);
        // A manual sync is always on the audit log; an automatic one only when it found something or failed, so
        // that a quiet day is not a wall of identical lines.
        if !auto || done.changed {
            let action = if auto { "sync.auto" } else { "sync" };
            toasts.extend(self.record(action, ok, &text));
        }
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
        v.sync_options = Sim::sync_options(self.interval_minutes);
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
        if let Some(outcome) = self.mapping_command(command.command()) {
            return outcome;
        }
        if let Command::SetSyncInterval(minutes) = command.command() {
            return self.set_sync_interval(*minutes);
        }
        match command.command() {
            Command::Sync => self.begin_sync(),
            _ => self.sim.dispatch(command),
        }
    }
    fn execute(&mut self, confirmed: Confirmed) -> Outcome {
        if let Some(outcome) = self.mapping_command(confirmed.command()) {
            return outcome;
        }
        self.sim.execute(confirmed)
    }
    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)> {
        let mut toasts = self.poll();
        if schedule::is_due(
            now(),
            self.last_attempt,
            self.interval_minutes,
            self.running.is_some(),
            self.failures,
        ) {
            self.start_sync(true);
        }
        // The simulation runs a minute in `MS_PER_MIN` simulated milliseconds (four real seconds in the
        // showcase). Here a minute is a real minute, so real time is scaled down to it: otherwise "Synced 1m
        // ago" would come every four seconds.
        let (sim_ms, carry) = to_sim_ms(self.carry_ms, millis);
        self.carry_ms = carry;
        toasts.extend(self.sim.tick(sim_ms));
        toasts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_minute_is_one_simulated_minute_however_it_is_ticked() {
        let (mut carry, mut sim) = (0, 0);
        // 120 real seconds in half-second ticks.
        for _ in 0..240 {
            let (ms, left) = to_sim_ms(carry, 500);
            sim += ms;
            carry = left;
        }
        assert_eq!(
            sim / MS_PER_MIN,
            2,
            "two minutes, not 30 (four seconds each)"
        );
    }
}

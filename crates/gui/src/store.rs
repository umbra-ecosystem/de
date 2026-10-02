//! The [`Store`] over the engine.
//!
//! Tickets, sync and the workspace lifecycle are real: tickets come from the Jira mirror and local
//! tracking, "Sync now" runs the `acli` adapter on a worker thread, and opening, closing or
//! switching a workspace runs the real steps of `docs/design/workspace-lifecycle.md` off the UI
//! thread. Everything else the views ask for is served by the widgets' simulation running on those
//! tickets, until the engine serves it.

use std::str::FromStr;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use de_core::config::Config;
use de_core::store::{Kind, Store as Db};
use de_core::synclog;
use de_core::types::Slug;
use de_core::workspace::health::Overall;
use de_core::workspace::{Workspace, registry, startup_order};
use de_widgets::sim::{INIT_COMMAND, Sim, ago};
use de_widgets::sim::model::MS_PER_MIN;
use de_widgets::vm::*;
use de_widgets::{Outcome, ReviewSel, Store};

use crate::lifecycle::{self, Seq};
use crate::measure::{self, Measured};
use crate::sync::{SyncDone, last_sync_minutes_ago, load_cached, sync_real};
use crate::{audit, localtime, logs, mapping, schedule};

/// How often the services, git state and active ticket are read again.
const MEASURE_MS: i64 = 30_000;
/// How often the list of saved workspaces is re-read (it is only a handful of small files).
const SAVED_MS: i64 = 5_000;
/// What any workspace command says while one is already opening, closing or switching.
const BUSY: &str = "A workspace is being opened or closed. Wait for it to finish.";

/// One saved workspace: its name, its projects in the order they start, and when it was last used.
struct SavedWs {
    name: Slug,
    projects: Vec<Slug>,
    last_used: Option<i64>,
}

/// A sync in flight, and whose tickets it is reading — so a result that belongs to a workspace
/// that is no longer open is dropped instead of shown.
struct RunningSync {
    scope: Option<Slug>,
    auto: bool,
    rx: Receiver<SyncDone>,
}

pub struct CoreStore {
    sim: Sim,
    running: Option<RunningSync>,
    /// Real milliseconds not yet turned into simulated ones (see [`CoreStore::tick`]).
    carry_ms: i64,
    /// Minutes between automatic syncs (0 is off), when the last sync was started (unix seconds), how many in a
    /// row failed. See [`crate::schedule`].
    interval_minutes: u32,
    last_attempt: i64,
    failures: u32,
    /// The workspace open in this window; every database the store touches is that
    /// workspace's own (`None` before any workspace is open).
    workspace: Option<Slug>,
    /// The saved workspaces, re-read now and then.
    saved: Vec<SavedWs>,
    /// The last reading of the open workspace and the saved list; `None` until one lands.
    measured: Option<Measured>,
    /// The reading in flight, and whether the person asked for it (only they get a toast).
    measure: Option<Receiver<Measured>>,
    measure_manual: bool,
    since_measure: i64,
    since_saved: i64,
    /// The opening, closing or switching sequence running now.
    lifecycle: Option<Seq>,
    /// A start or stop of the whole workspace's services in flight.
    spin: Option<Receiver<lifecycle::SpinDone>>,
    /// Toasts earned by something that ran inside a dispatch; the next tick hands them over.
    queued: Vec<(String, ToastKind)>,
}

/// Real milliseconds (plus what was carried over) as simulated milliseconds, and what is left over. A real minute
/// is `MS_PER_MIN` simulated milliseconds.
fn to_sim_ms(carry: i64, real: i64) -> (i64, i64) {
    let per_sim_ms = 60_000 / MS_PER_MIN;
    let total = carry + real;
    (total / per_sim_ms, total % per_sim_ms)
}

/// The audit log of `workspace`'s `state.db`, as the window's entries.
fn stored_audit(workspace: Option<&Slug>) -> eyre::Result<Vec<de_widgets::sim::model::AuditEntry>> {
    audit::entries(&Db::open_scoped_for(workspace, Kind::State)?, now())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl CoreStore {
    /// Starts from the cached tickets of the open workspace and looks for new ones right away.
    ///
    /// The workspace this folder or `config.toml` points at is already open: the app never
    /// starts on the list when it knows which workspace was being used.
    pub fn open() -> Self {
        let workspace = Workspace::active().ok().flatten().map(|w| w.config().name.clone());
        let scope = workspace.as_ref();
        let cached = load_cached(scope).unwrap_or_default();
        let mut sim = Sim::with_tickets(cached);
        sim.set_last_sync_minutes_ago(last_sync_minutes_ago(scope, now()));
        sim.set_start_minute_of_day(localtime::minute_of_day(now()));
        // Pull requests come from GitHub, which the app does not read yet: nothing is said about them.
        sim.set_github_ready(false);
        sim.set_wall_clock_start(now());
        // The audit log is on disk; the simulation only shows it.
        let stored = stored_audit(scope).unwrap_or_default();
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
            workspace,
            saved: Vec::new(),
            measured: None,
            measure: None,
            measure_manual: false,
            since_measure: 0,
            since_saved: 0,
            lifecycle: None,
            spin: None,
            queued: Vec::new(),
        };
        store.refresh_saved();
        store.start_measure();
        store.begin_sync();
        store
    }

    /// Write one line to the audit log and show it. A log that cannot be written is reported to the user, since
    /// an action nobody can see afterwards is a problem of its own.
    fn record(&mut self, action: &str, ok: bool, text: &str) -> Option<(String, ToastKind)> {
        let saved = Db::open_scoped_for(self.workspace.as_ref(), Kind::State)
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
        let scope = self.workspace.clone();
        let worker_scope = scope.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(sync_real(worker_scope.as_ref(), now()));
        });
        self.running = Some(RunningSync { scope, auto, rx });
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
                if let Ok(fresh) = load_cached(self.workspace.as_ref()) {
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
        let polled = match &self.running {
            None => return Vec::new(),
            Some(run) => (run.scope.clone(), run.auto, run.rx.try_recv()),
        };
        let (scope, auto, got) = polled;
        // The window moved on while this sync was running: its tickets belong to the workspace
        // that was open when it started, so nothing of them is shown here.
        if scope != self.workspace {
            self.running = None;
            return Vec::new();
        }
        let done = match got {
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

    /* ------------------------ workspaces ------------------------ */

    /// Re-read the saved workspaces: their names, their projects in the order they start, and when
    /// each was last used. The simulation's own list follows, so the palette offers real names.
    fn refresh_saved(&mut self) {
        let list = registry::list_workspaces().unwrap_or_default();
        self.saved = list
            .entries
            .iter()
            .map(|entry| SavedWs {
                name: entry.name.clone(),
                projects: registry::load_workspace(&entry.name)
                    .map(|ws| startup_order(&ws))
                    .unwrap_or_default(),
                last_used: entry.last_used,
            })
            .collect();
        let names: Vec<WorkspaceName> = self
            .saved
            .iter()
            .map(|s| WorkspaceName::new(s.name.as_str()))
            .collect();
        let open = self
            .workspace
            .as_ref()
            .map(|name| WorkspaceName::new(name.as_str()));
        self.sim.replace_workspaces(names, open.as_ref());
    }

    /// The saved workspaces as list rows, most recently used first — the open one counting as used
    /// now, and ties keeping the registry's order (which is by name).
    fn workspace_items(&self) -> Vec<WorkspaceItemVm> {
        let mut ordered: Vec<&SavedWs> = self.saved.iter().collect();
        ordered.sort_by_key(|s| {
            let used = if self.workspace.as_ref() == Some(&s.name) {
                i64::MAX
            } else {
                s.last_used.unwrap_or(i64::MIN)
            };
            std::cmp::Reverse(used)
        });
        ordered
            .into_iter()
            .map(|s| {
                let current = self.workspace.as_ref() == Some(&s.name);
                WorkspaceItemVm {
                    name: WorkspaceName::new(s.name.as_str()),
                    projects: s.projects.iter().map(|p| p.to_string()).collect(),
                    repos: s.projects.len() as u32,
                    up: self.up_dot(&s.name),
                    current,
                    last_used: match (current, s.last_used) {
                        (true, _) => "open".to_string(),
                        (false, None) => "never".to_string(),
                        (false, Some(at)) => ago((now() - at).max(0)),
                    },
                }
            })
            .collect()
    }

    /// Whether docker last said this workspace has at least one service running.
    fn up_dot(&self, name: &Slug) -> bool {
        self.measured
            .as_ref()
            .is_some_and(|m| m.running.get(name).copied().unwrap_or_default())
    }

    /// Read the open workspace and the saved list on a worker thread; the answer comes back on a
    /// later tick. Nothing here waits on docker or git.
    fn start_measure(&mut self) {
        if self.measure.is_some() {
            return;
        }
        // Nothing is in flight, so any earlier request has been answered: the reading that starts
        // now is one nobody asked for, and it stays quiet.
        self.measure_manual = false;
        let scope = self.workspace.clone();
        let saved: Vec<Slug> = self.saved.iter().map(|s| s.name.clone()).collect();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(measure::measure(scope.as_ref(), &saved));
        });
        self.measure = Some(rx);
        self.since_measure = 0;
    }

    /// Apply the reading when it lands. Only a check the person asked for says anything.
    fn poll_measure(&mut self) -> Vec<(String, ToastKind)> {
        let got = match &self.measure {
            None => return Vec::new(),
            Some(rx) => rx.try_recv(),
        };
        let measured = match got {
            Ok(measured) => measured,
            Err(TryRecvError::Empty) => return Vec::new(),
            Err(TryRecvError::Disconnected) => {
                self.measure = None;
                return Vec::new();
            }
        };
        let manual = std::mem::take(&mut self.measure_manual);
        let said = manual.then(|| match &measured.health {
            Some(health) if matches!(health.overall(), Overall::Unavailable) => (
                health
                    .exception()
                    .unwrap_or_else(|| "Docker could not be asked.".to_string()),
                ToastKind::Bad,
            ),
            _ => ("Services checked".to_string(), ToastKind::Info),
        });
        self.measure = None;
        self.since_measure = 0;
        self.measured = Some(measured);
        said.into_iter().collect()
    }

    /// Start or stop the open workspace's services on a worker thread; the result is a toast on a
    /// later tick, never a wait.
    fn spin_services(&mut self, up: bool) -> Outcome {
        if self.spin.is_some() {
            return Outcome::fail(if up {
                "The services are already starting."
            } else {
                "The services are already stopping."
            });
        }
        if self.lifecycle.is_some() {
            return Outcome::fail(BUSY);
        }
        let Some(scope) = self.workspace.clone() else {
            return Outcome::ok();
        };
        let workspace = match registry::load_workspace(&scope) {
            Ok(workspace) => workspace,
            Err(e) => return Outcome::fail(e.to_string()),
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(lifecycle::spin(&workspace, up));
        });
        self.spin = Some(rx);
        Outcome::ok()
    }

    /// Apply a finished start or stop: say what happened, then look at the services again.
    fn poll_spin(&mut self) -> Vec<(String, ToastKind)> {
        let got = match &self.spin {
            None => return Vec::new(),
            Some(rx) => rx.try_recv(),
        };
        let done = match got {
            Ok(done) => done,
            Err(TryRecvError::Empty) => return Vec::new(),
            Err(TryRecvError::Disconnected) => {
                self.spin = None;
                return vec![(
                    "The services stopped changing unexpectedly. Press Refresh to see where they stand."
                        .to_string(),
                    ToastKind::Warn,
                )];
            }
        };
        self.spin = None;
        self.start_measure();
        let mut toasts = Vec::new();
        match done.error {
            Some(why) => toasts.push((why, ToastKind::Bad)),
            None => {
                let (action, said) = if done.up {
                    ("workspace.start", "Workspace started")
                } else {
                    ("workspace.stop", "Workspace stopped")
                };
                toasts.extend(self.record(action, true, said));
                toasts.push((said.to_string(), ToastKind::Ok));
            }
        }
        toasts
    }

    /// Ask the engine again how the services stand; the answer comes back on a later tick.
    fn refresh_health(&mut self) -> Outcome {
        if self.lifecycle.is_some() {
            return Outcome::fail(BUSY);
        }
        self.start_measure();
        self.measure_manual = true;
        Outcome::ok()
    }

    /// Build the sequence for `kind`, reading both sides from disk. `Err` is the friendly reason
    /// it cannot start.
    fn start_sequence(&mut self, kind: lifecycle::Kind) -> Result<(), String> {
        let current = match &kind {
            lifecycle::Kind::Close { from } | lifecycle::Kind::Switch { from, .. } => {
                registry::load_workspace(from).ok()
            }
            lifecycle::Kind::Open { .. } => None,
        };
        let target = match &kind {
            lifecycle::Kind::Open { to } | lifecycle::Kind::Switch { to, .. } => {
                Some(registry::load_workspace(to).map_err(|e| e.to_string())?)
            }
            lifecycle::Kind::Close { .. } => None,
        };
        self.lifecycle = Some(Seq::build(kind, current.as_ref(), target.as_ref())?);
        Ok(())
    }

    /// Open `name`: close what is open first, as the design says, and show every step of it.
    fn select_workspace(&mut self, name: WorkspaceName) -> Outcome {
        if self.lifecycle.is_some() {
            return Outcome::fail(BUSY);
        }
        let to = match Slug::from_str(name.as_str()) {
            Ok(to) => to,
            Err(why) => return Outcome::fail(format!("'{name}' is not a workspace name: {why}")),
        };
        if self.workspace.as_ref() == Some(&to) {
            return Outcome::ok();
        }
        let kind = match &self.workspace {
            Some(from) => lifecycle::Kind::Switch {
                from: from.clone(),
                to,
            },
            None => lifecycle::Kind::Open { to },
        };
        match self.start_sequence(kind) {
            Ok(()) => Outcome::ok(),
            Err(why) => Outcome::fail(why),
        }
    }

    /// Close the open workspace, showing every step of it.
    fn close_workspace(&mut self) -> Outcome {
        if self.lifecycle.is_some() {
            return Outcome::fail(BUSY);
        }
        let Some(from) = self.workspace.clone() else {
            return Outcome::ok();
        };
        match self.start_sequence(lifecycle::Kind::Close { from }) {
            Ok(()) => Outcome::ok(),
            Err(why) => Outcome::fail(why),
        }
    }

    /// Carry on past a failure: apply whatever has been done so far.
    fn continue_sequence(&mut self) -> Outcome {
        let ready = self
            .lifecycle
            .as_ref()
            .is_some_and(|seq| seq.state() == SequenceState::NeedsDecision && !seq.blocked());
        if ready {
            self.finish_sequence();
        }
        Outcome::ok()
    }

    /// Run the sequence again from its first step.
    fn retry_sequence(&mut self) -> Outcome {
        let Some(kind) = self.lifecycle.as_ref().map(|seq| seq.kind().clone()) else {
            return Outcome::ok();
        };
        match self.start_sequence(kind) {
            Ok(()) => Outcome::ok(),
            Err(why) => Outcome::fail(why),
        }
    }

    /// Leave the sequence where it stands: the window keeps the workspace it had.
    fn abort_sequence(&mut self) -> Outcome {
        let decided = self
            .lifecycle
            .as_ref()
            .is_some_and(|seq| seq.state() == SequenceState::NeedsDecision);
        if decided {
            self.lifecycle = None;
            self.queued.push((
                "Stopped. Nothing in the window changed; services that were started keep running."
                    .to_string(),
                ToastKind::Info,
            ));
        }
        Outcome::ok()
    }

    /// Apply what a finished sequence was for. The workspace open in the window changes here,
    /// which the session notices and resets around.
    fn finish_sequence(&mut self) {
        let Some(seq) = self.lifecycle.take() else {
            return;
        };
        let toasts = match seq.kind().clone() {
            lifecycle::Kind::Open { to } => self.apply_open(to),
            lifecycle::Kind::Close { from } => self.apply_close(from, true),
            lifecycle::Kind::Switch { from, to } => {
                let closed = self.apply_close(from, false);
                let mut toasts = self.apply_open(to);
                // A switch says one thing: where it ended up. Only a warning from closing is kept.
                toasts.extend(
                    closed
                        .into_iter()
                        .filter(|(_, kind)| *kind != ToastKind::Info),
                );
                toasts
            }
        };
        self.queued.extend(toasts);
    }

    /// The workspace is open now: record it as active, load what it shows, start fresh on it.
    fn apply_open(&mut self, to: Slug) -> Vec<(String, ToastKind)> {
        let recorded = registry::select_workspace(&to, now());
        self.workspace = Some(to.clone());
        // A sync started for the workspace that was open before must not land in this one.
        if self
            .running
            .as_ref()
            .is_some_and(|run| run.scope.as_ref() != Some(&to))
        {
            self.running = None;
        }
        self.failures = 0;
        let mut toasts = self.load_workspace_data(&to);
        toasts.extend(self.record("workspace.open", true, to.as_str()));
        self.refresh_saved();
        self.start_measure();
        self.begin_sync();
        let name = to.to_string();
        match recorded {
            Ok(_) => toasts.push((format!("Opened {name}"), ToastKind::Ok)),
            Err(e) => toasts.push((
                format!(
                    "Opened {name}, but the active workspace could not be recorded in \
                     config.toml: {e:#}. Run `de workspace select {name}` to record it."
                ),
                ToastKind::Warn,
            )),
        }
        toasts
    }

    /// Nothing of `from` may stay in the window: its tickets, its audit log and the sync aimed at
    /// it. It is also no longer the active workspace, so the next launch shows the list again.
    fn apply_close(&mut self, from: Slug, announce: bool) -> Vec<(String, ToastKind)> {
        let mut toasts: Vec<(String, ToastKind)> = self.record("workspace.close", true, from.as_str()).into_iter().collect();
        self.workspace = None;
        self.measured = None;
        self.sim.replace_tickets(Vec::new());
        self.sim.set_last_sync_minutes_ago(None);
        self.sim.set_last_sync_report(Vec::new());
        self.sim.use_external_audit(Vec::new());
        // A sync for the workspace that just closed must not land anywhere afterwards.
        self.running = None;
        self.refresh_saved();
        self.start_measure();
        if registry::deselect_workspace().is_err() {
            toasts.push((
                format!(
                    "Closed {from}, but the active workspace could not be cleared in config.toml. \
                     Run `de config active --unset` to clear it."
                ),
                ToastKind::Warn,
            ));
        }
        if announce {
            toasts.push((format!("Closed {from}"), ToastKind::Info));
        }
        toasts
    }

    /// Read what the workspace shows: its tickets, when they were last synced, and its audit log.
    fn load_workspace_data(&mut self, scope: &Slug) -> Vec<(String, ToastKind)> {
        let mut toasts = Vec::new();
        match load_cached(Some(scope)) {
            Ok(fresh) => self.sim.replace_tickets(fresh),
            Err(e) => toasts.push((
                format!(
                    "Could not read the workspace's tickets: {e:#}. \
                     Check that the data folder is writable."
                ),
                ToastKind::Bad,
            )),
        }
        self.sim
            .set_last_sync_minutes_ago(last_sync_minutes_ago(Some(scope), now()));
        match stored_audit(Some(scope)) {
            Ok(entries) => {
                self.sim
                    .set_last_sync_report(audit::last_sync_report(&entries));
                self.sim.use_external_audit(entries);
            }
            Err(e) => toasts.push((
                format!(
                    "Could not read the audit log: {e:#}. Check that the data folder is readable."
                ),
                ToastKind::Warn,
            )),
        }
        toasts
    }

    /// What stopping the open workspace's services will actually run, so the confirmation never
    /// names projects this workspace does not have.
    fn stop_preview(&self) -> Preview {
        let mut preview = Preview::new(
            Command::StopWorkspace,
            "Stop the workspace",
            Risk::Medium,
            "Stops every service. Uncommitted work stays on disk.",
            "Stop",
        );
        let order: Vec<String> = self
            .workspace
            .as_ref()
            .and_then(|name| registry::load_workspace(name).ok())
            .map(|ws| {
                lifecycle::service_steps(&ws, false)
                    .into_iter()
                    .map(|s| s.project.to_string())
                    .collect()
            })
            .unwrap_or_default();
        if !order.is_empty() {
            preview.payload = vec![format!(
                "docker compose down (dependency order: {})",
                order.join(" \u{2192} ")
            )];
        }
        preview.facts = self.dirty_facts();
        preview
    }

    /// The repos with work the confirmation lists, so nothing is stopped without saying so.
    fn dirty_facts(&self) -> Vec<String> {
        self.measured
            .as_ref()
            .into_iter()
            .flat_map(|m| &m.git)
            .filter_map(|(project, read)| {
                let status = read.as_ref().ok()?;
                (!status.is_clean()).then(|| format!("{project} has uncommitted changes"))
            })
            .collect()
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
        let projects = self
            .workspace
            .as_ref()
            .and_then(|name| self.saved.iter().find(|s| &s.name == name))
            .map(|s| s.projects.as_slice())
            .unwrap_or_default();
        measure::workspace_vm(self.workspace.as_ref(), projects, self.measured.as_ref())
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
    fn workspaces(&self) -> WorkspacesVm {
        WorkspacesVm {
            current: self
                .workspace
                .as_ref()
                .map(|name| WorkspaceName::new(name.as_str())),
            items: self.workspace_items(),
        }
    }
    fn welcome(&self) -> WelcomeVm {
        WelcomeVm {
            providers: self.settings().providers,
            workspaces: self.workspace_items(),
            init_command: INIT_COMMAND.to_string(),
            current: self
                .workspace
                .as_ref()
                .map(|name| WorkspaceName::new(name.as_str())),
        }
    }
    fn sequence(&self) -> Option<SequenceVm> {
        self.lifecycle.as_ref().map(Seq::vm)
    }
    fn all_workspaces(&self, query: &str) -> AllWorkspacesVm {
        let items = self.workspace_items();
        let q = query.trim().to_lowercase();
        let total = items.len();
        AllWorkspacesVm {
            query: query.to_string(),
            items: items
                .into_iter()
                .filter(|w| {
                    q.is_empty()
                        || w.name.to_lowercase().contains(&q)
                        || w.projects.iter().any(|p| p.to_lowercase().contains(&q))
                })
                .collect(),
            total,
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
        if matches!(command.command(), Command::StopWorkspace) {
            return Some(self.stop_preview());
        }
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
            Command::SelectWorkspace(name) => self.select_workspace(name.clone()),
            Command::CloseWorkspace => self.close_workspace(),
            Command::ContinueSequence => self.continue_sequence(),
            Command::RetrySequence => self.retry_sequence(),
            Command::AbortSequence => self.abort_sequence(),
            Command::RefreshHealth => self.refresh_health(),
            Command::StartWorkspace => self.spin_services(true),
            _ => self.sim.dispatch(command),
        }
    }
    fn execute(&mut self, confirmed: Confirmed) -> Outcome {
        if let Some(outcome) = self.mapping_command(confirmed.command()) {
            return outcome;
        }
        // Stopping is guarded, so it arrives here rather than in `dispatch`.
        if matches!(confirmed.command(), Command::StopWorkspace) {
            return self.spin_services(false);
        }
        self.sim.execute(confirmed)
    }
    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)> {
        let mut toasts = std::mem::take(&mut self.queued);
        toasts.extend(self.poll());
        toasts.extend(self.poll_measure());
        toasts.extend(self.poll_spin());
        // The steps of an open or close ran off the UI thread; when their "Ready" beat is over,
        // the workspace actually changes (which the session notices and resets around).
        let finished = self
            .lifecycle
            .as_mut()
            .is_some_and(|seq| seq.tick(millis));
        if finished {
            self.finish_sequence();
        }
        // How the services stand is re-read now and then, and after anything that changes them.
        self.since_measure += millis;
        if self.since_measure >= MEASURE_MS
            && self.measure.is_none()
            && self.lifecycle.is_none()
        {
            self.start_measure();
        }
        // The saved workspaces are only a handful of small files, so re-reading them is cheap.
        self.since_saved += millis;
        if self.since_saved >= SAVED_MS {
            self.since_saved = 0;
            self.refresh_saved();
        }
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
    use de_core::git::RepoStatus;

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

    fn slug(text: &str) -> Slug {
        Slug::from_str(text).unwrap()
    }

    fn saved(name: &str, projects: &[&str], last_used: Option<i64>) -> SavedWs {
        SavedWs {
            name: slug(name),
            projects: projects.iter().map(|p| slug(p)).collect(),
            last_used,
        }
    }

    /// A store over the simulation, with nothing read from disk.
    fn store_with(
        saved: Vec<SavedWs>,
        workspace: Option<Slug>,
        measured: Option<Measured>,
    ) -> CoreStore {
        CoreStore {
            sim: Sim::new(),
            running: None,
            carry_ms: 0,
            interval_minutes: 0,
            last_attempt: 0,
            failures: 0,
            workspace,
            saved,
            measured,
            measure: None,
            measure_manual: false,
            since_measure: 0,
            since_saved: 0,
            lifecycle: None,
            spin: None,
            queued: Vec::new(),
        }
    }

    fn selecting(name: &str) -> LocalCommand {
        match Command::SelectWorkspace(WorkspaceName::new(name)).classify() {
            Classified::Local(local) => local,
            _ => panic!("choosing a workspace writes nothing remote"),
        }
    }

    fn stopping() -> LocalCommand {
        match Command::StopWorkspace.classify() {
            Classified::Local(local) => local,
            _ => panic!("docker is local"),
        }
    }

    /// The lists are most recently used first, with the open one counting as used now — so it
    /// leads — and a workspace nobody has opened saying so rather than a number.
    #[test]
    fn the_saved_workspaces_are_ordered_by_last_use_and_the_open_one_says_open() {
        let now = now();
        let store = store_with(
            vec![
                saved("alpha", &["web"], Some(now - 3 * 3600)),
                saved("beta", &["api", "web"], Some(now - 60)),
                saved("gamma", &[], None),
            ],
            Some(slug("beta")),
            None,
        );

        let items = store.workspaces().items;
        let names: Vec<&str> = items.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["beta", "alpha", "gamma"]);
        assert_eq!(items[0].last_used, "open");
        assert!(items[0].current);
        assert_eq!(items[0].repos, 2, "both of its projects");
        assert_eq!(items[1].last_used, "3h ago");
        assert_eq!(items[2].last_used, "never");
        assert!(
            !items[0].up,
            "no reading has landed, so nothing is claimed about its services"
        );
    }

    /// The page before the dock is the same list, plus the command that makes one.
    #[test]
    fn the_welcome_page_lists_the_saved_workspaces_and_how_to_make_one() {
        let store = store_with(vec![saved("shop", &["web"], None)], None, None);
        let welcome = store.welcome();
        assert_eq!(welcome.init_command, "de init");
        assert_eq!(welcome.current, None);
        assert_eq!(welcome.workspaces.len(), 1);
        assert_eq!(welcome.workspaces[0].name.as_str(), "shop");
    }

    /// Stopping is confirmed before anything runs, and the confirmation names this workspace's
    /// services and the work that would be left behind — never the simulation's invented repos.
    #[test]
    fn the_stop_confirmation_only_names_this_workspaces_services_and_work_on_disk() {
        let mut store = store_with(Vec::new(), None, None);
        let preview = store
            .local_guard(&stopping())
            .expect("stopping is always confirmed first");
        assert_eq!(preview.title, "Stop the workspace");
        assert!(preview.payload.is_empty(), "no workspace, no order to run");

        let dirty = RepoStatus {
            branch: Some("develop".to_string()),
            detached: false,
            head: Some("abc1234".to_string()),
            modified: 0,
            staged: 0,
            untracked: 3,
            upstream: None,
            ahead: 0,
            behind: 0,
            unpushed: 0,
        };
        store.measured = Some(Measured {
            git: vec![(slug("web"), Ok(dirty))],
            ..Measured::default()
        });
        let preview = store.local_guard(&stopping()).unwrap();
        assert_eq!(preview.facts, vec!["web has uncommitted changes"]);
    }

    /// One sequence at a time: while one is running nothing else may start, and the person is
    /// told why rather than left wondering.
    #[test]
    fn nothing_starts_while_one_sequence_is_already_running() {
        let mut store = store_with(Vec::new(), None, None);
        store.lifecycle = Some(
            lifecycle::Seq::build(lifecycle::Kind::Close { from: slug("shop") }, None, None)
                .unwrap(),
        );

        let outcome = store.dispatch(selecting("shop"));
        assert_eq!(outcome, Outcome::fail(BUSY));
    }

    /// A workspace that is not saved cannot be opened, and the refusal says where to look instead.
    #[test]
    fn choosing_a_workspace_that_is_not_saved_says_what_to_run_instead() {
        let name = "de-gui-no-such-workspace-9f3k2";
        if registry::load_workspace(&slug(name)).is_ok() {
            return;
        }
        let mut store = store_with(Vec::new(), None, None);

        let Outcome::Refused(reasons) = store.dispatch(selecting(name)) else {
            panic!("a workspace nobody saved was opened");
        };
        assert!(
            reasons[0].contains("`de workspace list`") && reasons[0].contains("`de init`"),
            "{}",
            reasons[0]
        );
        assert!(store.sequence().is_none(), "nothing was started");
    }
}

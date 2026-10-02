//! An in-memory stand-in for `de-core`: the lifecycle the real core exposes, fake sync, fake Actions runs and the
//! next-action rules. A port of the prototype's `logic.js`. No I/O, no network, no engine.
//!
//! Nothing outside `sim` knows these types; the views see only [`crate::vm`] values through [`crate::Store`].

mod data;
mod detail;
mod lifecycle;
pub mod model;
mod queries;
mod ranking;
mod rules;
mod sequence;
mod store_impl;
mod worlds;

pub use rules::{Rule, Sug};
pub use worlds::{INIT_COMMAND, open_intent};

use std::collections::BTreeMap;

use model::*;

use crate::vm::{Branch, MappingKey, RepoName, TicketKey, ToastKind};

pub struct Sim {
    pub(crate) ms: i64,
    /// The minute of the day (0..1440) that `ms == 0` is, so a clock reads as real time where it should.
    pub(crate) start_min: i64,
    /// The unix time that `ms == 0` is, for a store that runs on real time; ages ("2d ago") need it.
    pub(crate) wall_start: Option<i64>,
    pub(crate) seq: u64,
    pub(crate) run: u32,
    pub(crate) sync: SyncState,
    pub(crate) jira_ready: bool,
    pub(crate) gh_ready: bool,
    pub(crate) locks: BTreeMap<RepoName, Lock>,
    pub(crate) sims: Sims,
    pub(crate) ws: Workspace,
    pub(crate) wait_min: u32,
    /// Minutes between automatic syncs (0 is off) and after how many minutes without one the window calls the
    /// data stale.
    pub(crate) sync_minutes: u32,
    pub(crate) stale_sync_min: i64,
    /// The editable Jira mapping; a key that is absent is unset.
    pub(crate) mapping: BTreeMap<MappingKey, String>,
    pub(crate) tickets: Vec<Ticket>,
    pub(crate) audit: Vec<AuditEntry>,
    /// The audit log is kept elsewhere (a real store persists it); the simulation then neither adds to it nor
    /// loses it.
    pub(crate) audit_external: bool,
    /// The saved workspaces and which one is open (its world is the live one above); see `worlds`.
    pub(crate) saved: Vec<worlds::Saved>,
    pub(crate) open_ws: Option<usize>,
    /// Opening or closing a workspace in progress, or waiting for a decision; see `sequence`.
    pub(crate) wseq: Option<sequence::Seq>,
    pub(crate) responses: Responses,
    pub(crate) repos: Vec<RepoCfg>,
    pub(crate) toasts: Vec<(String, ToastKind)>,
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    pub fn new() -> Self {
        let branches = [
            ("api-client", "develop"),
            ("web", "wip/my-experiment"),
            ("worker", "develop"),
            ("docs", "master"),
        ]
        .into_iter()
        .map(|(a, b)| (RepoName::from(a), Branch::from(b)))
        .collect();
        let mut sim = Self {
            ms: 0,
            start_min: START_MIN,
            wall_start: None,
            seq: 100,
            run: 480,
            sync: SyncState {
                last_ok: -2 * MS_PER_MIN,
                last_attempt: -2 * MS_PER_MIN,
                ..SyncState::default()
            },
            jira_ready: true,
            gh_ready: true,
            locks: BTreeMap::new(),
            sims: Sims::default(),
            ws: Workspace {
                branches,
                dirty: [RepoName::from("web")].into_iter().collect(),
                up: true,
            },
            wait_min: 30,
            sync_minutes: 10,
            stale_sync_min: 6,
            mapping: [
                (MappingKey::ReviewJql, "project = PROJ AND status = \"In Review\""),
                (MappingKey::AccountId, "acct-0001"),
            ]
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect(),
            tickets: data::seed_tickets(),
            audit: Vec::new(),
            audit_external: false,
            saved: worlds::seed_saved(),
            open_ws: Some(0),
            wseq: None,
            responses: Responses::new(),
            repos: repos(),
            toasts: Vec::new(),
        };
        sim.observe();
        sim
    }

    /// A simulation that starts from these tickets instead of the invented ones (the real store uses it for what
    /// the engine does not serve yet: PRs, review and shipping stay simulated).
    pub fn with_tickets(tickets: Vec<Ticket>) -> Self {
        let mut sim = Self {
            tickets,
            ..Self::new()
        };
        sim.observe();
        sim
    }

    /// Swap in freshly read tickets. A ticket already known keeps its simulated local state; the Jira-side
    /// fields come from the new one.
    pub fn replace_tickets(&mut self, fresh: Vec<Ticket>) {
        let mut old = std::mem::take(&mut self.tickets);
        self.tickets = fresh
            .into_iter()
            .map(|mut t| {
                if let Some(i) = old.iter().position(|o| o.key == t.key) {
                    let o = old.swap_remove(i);
                    if t.stage.local().is_none() {
                        t.stage = o.stage;
                    }
                }
                t
            })
            .collect();
        self.observe();
    }

    /// The saved workspaces as the engine has them: the invented ones are not what the app opens.
    /// `open` is the one this window has open, if any.
    ///
    /// Nothing of the live workspace (its tickets especially) is touched: the caller loads what
    /// the workspace being opened shows.
    pub fn replace_workspaces(
        &mut self,
        names: Vec<crate::vm::WorkspaceName>,
        open: Option<&crate::vm::WorkspaceName>,
    ) {
        self.saved = names
            .into_iter()
            .map(|name| worlds::Saved {
                name,
                last_used: None,
                world: None,
            })
            .collect();
        self.open_ws = open.and_then(|o| self.saved.iter().position(|s| s.name == *o));
        self.wseq = None;
    }

    /// A real sync is under way: the status bar shows it and the fake completion never fires.
    pub fn sync_began(&mut self) {
        self.sync.running = true;
        self.sync.last_attempt = self.ms;
        self.sync.finish_at = None;
    }

    /// A real sync ended. `report` is `(ok, source, text)` per source; any failed one makes it partial.
    pub fn sync_ended(&mut self, report: Vec<(bool, String, String)>) {
        self.sync.running = false;
        self.sync.finish_at = None;
        let failed = report.iter().any(|r| !r.0);
        if !failed {
            self.sync.last_ok = self.ms;
            self.sync.never = false;
        }
        self.sync.last_error = failed.then(|| "partial".to_string());
        let text = report
            .iter()
            .map(|r| r.2.clone())
            .collect::<Vec<_>>()
            .join("; ");
        self.sync.report = report;
        self.audit(
            "sync",
            None,
            None,
            if failed {
                AuditOutcome::Failure
            } else {
                AuditOutcome::Success
            },
            &text,
        );
    }

    /// The last successful sync was `minutes` ago, or there has been none. The real store seeds this at start;
    /// the simulation starts with two minutes.
    pub fn set_last_sync_minutes_ago(&mut self, minutes: Option<i64>) {
        self.sync.never = minutes.is_none();
        let ms = minutes.unwrap_or(0) * MS_PER_MIN;
        self.sync.last_ok = self.ms - ms;
        self.sync.last_attempt = self.ms - ms;
    }

    /// The status bar's words for a sync `minutes` minutes old: nothing under a minute is worth counting.
    pub fn synced_text(minutes: i64) -> String {
        match minutes {
            m if m < 1 => "Synced now".to_string(),
            m if m < 60 => format!("Synced {m}m ago"),
            m if m < 24 * 60 => format!("Synced {}h ago", m / 60),
            m => format!("Synced {}d ago", m / (24 * 60)),
        }
    }

    /// The simulation's zero is this unix time (and its minutes are real ones), so ages can be told.
    pub fn set_wall_clock_start(&mut self, unix: i64) {
        self.wall_start = Some(unix);
    }

    /// A unix time on the simulation's clock (milliseconds, negative before it started); `None` without a wall clock
    /// or without a time.
    pub(crate) fn ms_of(&self, unix: Option<i64>) -> Option<i64> {
        Some((unix? - self.wall_start?) * MS_PER_MIN / 60)
    }

    /// How long ago a unix time was, in the words of the lists (`5m ago`, `3h ago`, `2d ago`, `3w ago`, `4mo ago`,
    /// `1y ago`); `None` when there is no wall clock.
    pub(crate) fn ago_text(&self, at: i64) -> Option<String> {
        let now = self.wall_start? + self.ms.div_euclid(MS_PER_MIN) * 60;
        Some(ago(now - at))
    }

    /// What minute of the day it is now (local time), for the times the audit log and waits show.
    pub fn set_start_minute_of_day(&mut self, minute: i64) {
        self.start_min = minute.rem_euclid(24 * 60);
    }

    /// The interval the status and the stale-sync suggestion follow.
    pub fn set_sync_interval(&mut self, minutes: u32, stale_after_min: i64) {
        self.sync_minutes = minutes;
        self.stale_sync_min = stale_after_min;
    }

    /// Whether GitHub can be asked. When it cannot, whether a ticket has a pull request is unknown, so nothing is
    /// said about a missing one.
    pub fn set_github_ready(&mut self, ready: bool) {
        self.gh_ready = ready;
        self.observe();
    }

    pub fn set_jira_ready(&mut self, ready: bool) {
        self.jira_ready = ready;
    }

    /* ---------------------------- small helpers ---------------------------- */

    pub(crate) fn idx(&self, key: &TicketKey) -> Option<usize> {
        self.tickets.iter().position(|t| t.key == *key)
    }

    pub(crate) fn tk(&self, key: &TicketKey) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.key == *key)
    }

    pub(crate) fn repo(&self, name: &RepoName) -> &RepoCfg {
        self.repos
            .iter()
            .find(|r| r.name == *name)
            .unwrap_or(&self.repos[0])
    }

    pub(crate) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub(crate) fn clock(&self, at: Option<i64>) -> String {
        let ms = at.unwrap_or(self.ms);
        let m = self.start_min + ms.div_euclid(MS_PER_MIN);
        format!("{:02}:{:02}", (m / 60).rem_euclid(24), m.rem_euclid(60))
    }

    pub(crate) fn mins_ago(&self, ms: i64) -> i64 {
        ((self.ms - ms) / MS_PER_MIN).max(0)
    }

    /// Hand the audit log over to whoever stores it: from now on it is exactly what `set_audit` was given (newest
    /// first), and what the simulation would have recorded is dropped.
    pub fn use_external_audit(&mut self, entries: Vec<AuditEntry>) {
        self.audit_external = true;
        self.audit = entries;
    }

    /// What the side panel's "Last sync" shows before any sync in this session: the report of the last one on
    /// record. `(ok, source, text)` per source.
    pub fn set_last_sync_report(&mut self, report: Vec<(bool, String, String)>) {
        self.sync.report = report;
    }

    /// Replace the externally kept audit log (newest first).
    pub fn set_audit(&mut self, entries: Vec<AuditEntry>) {
        self.audit = entries;
    }

    pub(crate) fn audit(
        &mut self,
        action: &str,
        ticket: Option<&TicketKey>,
        repo: Option<&RepoName>,
        outcome: AuditOutcome,
        details: &str,
    ) {
        if self.audit_external {
            return;
        }
        let id = self.next_seq();
        let at = self.clock(None);
        self.audit.insert(
            0,
            AuditEntry {
                id,
                at,
                action: action.to_string(),
                ticket: ticket.cloned(),
                repo: repo.cloned(),
                outcome,
                details: details.to_string(),
            },
        );
    }

    pub(crate) fn ok(
        &mut self,
        action: &str,
        ticket: Option<&TicketKey>,
        repo: Option<&RepoName>,
        details: &str,
    ) {
        self.audit(action, ticket, repo, AuditOutcome::Success, details);
    }

    pub(crate) fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push((text.into(), kind));
    }

    pub(crate) fn hex(seed: &str) -> String {
        let mut h: u32 = 5381;
        for c in seed.chars() {
            h = h.wrapping_mul(33) ^ (c as u32);
        }
        format!("{h:08x}")[..7].to_string()
    }

    pub(crate) fn active_key(&self) -> Option<TicketKey> {
        self.tickets
            .iter()
            .find(|t| t.local() == Some(Local::Active))
            .map(|t| t.key.clone())
    }

    /// The first ticket in hand: claimed, in review or active. Parked and awaiting-alpha tickets do not count.
    pub(crate) fn in_hand(&self, except: &TicketKey) -> Option<&Ticket> {
        self.tickets.iter().find(|x| {
            x.key != *except
                && matches!(
                    x.local(),
                    Some(Local::Claimed | Local::Reviewing | Local::Active)
                )
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_in_the_largest_fitting_unit() {
        let cases = [
            (-5, "just now"),
            (0, "just now"),
            (59, "just now"),
            (60, "1m ago"),
            (59 * 60, "59m ago"),
            (3600, "1h ago"),
            (23 * 3600 + 3599, "23h ago"),
            (86_400, "1d ago"),
            (13 * 86_400, "13d ago"),
            (14 * 86_400, "2w ago"),
            (59 * 86_400, "8w ago"),
            (60 * 86_400, "2mo ago"),
            (364 * 86_400, "12mo ago"),
            (365 * 86_400, "1y ago"),
        ];
        for (secs, want) in cases {
            assert_eq!(ago(secs), want, "{secs}s");
        }
    }

    #[test]
    fn a_claim_row_carries_when_its_ticket_last_changed() {
        let mut sim = Sim::new();
        let claim = |sim: &Sim| {
            crate::Store::next(sim, true)
                .cards
                .into_iter()
                .find(|c| c.title == "Claim PROJ-139")
                .expect("the hotfix claim")
        };
        // The showcase has no wall clock: nothing to tell an age from.
        assert_eq!(claim(&sim).updated, None);

        sim.set_wall_clock_start(1_000_000);
        let i = sim.idx(&"PROJ-139".into()).unwrap();
        sim.tickets[i].updated_at = Some(1_000_000 - 2 * 86_400 - 60);
        sim.tickets[i].updated = "2026-09-29 10:00";
        let (short, full) = claim(&sim).updated.expect("an age");
        assert_eq!(short, "2d ago");
        assert_eq!(full, "Updated 2026-09-29 10:00");

        // The ticket tables carry the same age.
        let row = |sim: &Sim| {
            crate::Store::tickets(sim, crate::vm::Group::All)
                .sections
                .into_iter()
                .flat_map(|s| s.rows)
                .find(|r| r.key == "PROJ-139")
                .expect("the ticket in the list")
        };
        assert_eq!(row(&sim).updated.map(|u| u.0).as_deref(), Some("2d ago"));

        // A ticket whose update time is unknown shows none, rather than a made-up one.
        sim.tickets[i].updated_at = None;
        assert_eq!(claim(&sim).updated, None);
        assert_eq!(row(&sim).updated, None);
    }

    fn unclaimed_without_a_pr(sim: &Sim) -> TicketKey {
        sim.tickets
            .iter()
            .find(|t| t.local().is_none() && sim.pr_gap(t).is_some())
            .expect("the demo has an unclaimed ticket without a PR")
            .key
            .clone()
    }

    #[test]
    fn a_missing_pr_is_timed_from_when_it_was_first_seen_not_from_the_claim() {
        let mut sim = Sim::new();
        let key = unclaimed_without_a_pr(&sim);
        let seen = sim.tk(&key).unwrap().pr_wait.expect("noted when first seen, unclaimed");
        assert_eq!(seen.since, 0);
        assert!(sim.wait_left(Some(seen)).unwrap() > 0, "still inside the grace period");

        // Time passes: the stored fact does not change, only the time left derived from it does.
        sim.advance(10 * MS_PER_MIN);
        assert_eq!(sim.tk(&key).unwrap().pr_wait, Some(seen));
        let left = sim.wait_left(Some(seen)).unwrap();
        assert_eq!(left, i64::from(seen.mins - 10) * MS_PER_MIN);

        // Claiming does not start a new wait.
        let _ = crate::Store::dispatch(
            &mut sim,
            match crate::vm::Command::Claim(key.clone()).classify() {
                crate::vm::Classified::Local(l) => l,
                _ => unreachable!(),
            },
        );
        assert_eq!(sim.tk(&key).unwrap().pr_wait, Some(seen));

        // Once the grace period has passed, the ticket is no longer "awaiting" and no state had to change.
        sim.advance(i64::from(seen.mins) * MS_PER_MIN);
        assert_eq!(sim.tk(&key).unwrap().pr_wait, Some(seen));
        assert!(sim.pr_wait_expired(sim.tk(&key).unwrap()));
        assert!(
            sim.suggest()
                .iter()
                .filter(|s| s.ticket.as_ref() == Some(&key))
                .all(|s| !s.awaiting_pr)
        );
    }

    #[test]
    fn with_a_stored_first_seen_time_the_wait_counts_from_it_not_from_this_run() {
        let mut sim = Sim::new();
        sim.set_wall_clock_start(1_000_000);
        let key = unclaimed_without_a_pr(&sim);
        let i = sim.idx(&key).unwrap();

        // First seen in Review 20 minutes before this run began: 10 minutes of a 30 minute wait are left.
        sim.tickets[i].pr_wait = None;
        sim.tickets[i].status_since = Some(1_000_000 - 20 * 60);
        sim.observe();
        let w = sim.tk(&key).unwrap().pr_wait.unwrap();
        assert_eq!(sim.wait_left(Some(w)), Some(10 * MS_PER_MIN));

        // Seen two days ago: the wait is long over the moment it is looked at, and the ticket is not "awaiting".
        sim.tickets[i].pr_wait = None;
        sim.tickets[i].status_since = Some(1_000_000 - 2 * 86_400);
        sim.observe();
        let w = sim.tk(&key).unwrap().pr_wait.unwrap();
        assert!(sim.wait_left(Some(w)).unwrap() < 0);
        assert!(sim.pr_wait_expired(sim.tk(&key).unwrap()));
        assert!(sim.suggest().iter().filter(|s| s.ticket.as_ref() == Some(&key)).all(|s| !s.awaiting_pr));

        // A time in the future (a skewed clock) is not later than now.
        sim.tickets[i].pr_wait = None;
        sim.tickets[i].status_since = Some(1_000_000 + 3600);
        sim.observe();
        assert_eq!(sim.tk(&key).unwrap().pr_wait.map(|w| w.since), Some(0));
    }

    #[test]
    fn a_wait_is_forgotten_when_what_it_waited_for_goes_and_a_new_gap_is_timed_afresh() {
        let mut sim = Sim::new();
        let key = unclaimed_without_a_pr(&sim);
        sim.advance(5 * MS_PER_MIN);
        // GitHub cannot be asked: nothing is said about a missing pull request.
        sim.set_github_ready(false);
        assert_eq!(sim.tk(&key).unwrap().pr_wait, None);
        assert!(sim.suggest().iter().all(|s| !s.awaiting_pr));
        // Back again: the gap is new, so its clock is from now, not from five minutes ago.
        sim.set_github_ready(true);
        assert_eq!(sim.tk(&key).unwrap().pr_wait.map(|w| w.since), Some(5 * MS_PER_MIN));
    }

    #[test]
    fn unresolved_review_comments_are_timed_from_when_they_were_first_seen_too() {
        use crate::sim::model::Thread;
        let mut sim = Sim::new();
        // A ticket in review with a pull request and no open comment yet.
        let key = sim
            .tickets
            .iter()
            .find(|t| !t.prs.is_empty() && Sim::blocking_threads(t).is_empty() && Sim::thread_stage(t))
            .expect("a ticket in review with a pull request")
            .key
            .clone();
        assert_eq!(sim.tk(&key).unwrap().th_wait, None);

        // Three minutes in, a refresh shows an unresolved comment: that is when it was first seen.
        sim.advance(3 * MS_PER_MIN);
        let i = sim.idx(&key).unwrap();
        sim.tickets[i].prs[0].threads.push(Thread {
            id: crate::sim::model::ThreadId(9_001),
            file: "src/lib.rs".into(),
            line: None,
            author: "Dev".into(),
            text: "please rename".into(),
            resolved: false,
        });
        sim.observe();
        let seen = sim.tk(&key).unwrap().th_wait.expect("noted when first seen");
        assert_eq!(seen.since, 3 * MS_PER_MIN);

        // Time passing changes nothing stored; the time left is worked out from it.
        sim.advance(7 * MS_PER_MIN);
        assert_eq!(sim.tk(&key).unwrap().th_wait, Some(seen));
        assert_eq!(sim.wait_left(Some(seen)), Some(i64::from(seen.mins - 7) * MS_PER_MIN));

        // Resolved: forgotten.
        sim.tickets[i].prs[0].threads.last_mut().unwrap().resolved = true;
        sim.observe();
        assert_eq!(sim.tk(&key).unwrap().th_wait, None);
    }

    #[test]
    fn an_age_needs_a_wall_clock_and_moves_with_the_simulation_clock() {
        let mut sim = Sim::new();
        assert_eq!(sim.ago_text(1_000), None, "the showcase has no real time");
        sim.set_wall_clock_start(10_000);
        assert_eq!(sim.ago_text(10_000 - 7200).as_deref(), Some("2h ago"));
        sim.advance(3 * MS_PER_MIN);
        assert_eq!(sim.ago_text(10_000).as_deref(), Some("3m ago"));
    }

    #[test]
    fn the_sync_age_reads_now_under_a_minute_then_minutes_hours_days() {
        assert_eq!(Sim::synced_text(0), "Synced now");
        assert_eq!(Sim::synced_text(1), "Synced 1m ago");
        assert_eq!(Sim::synced_text(59), "Synced 59m ago");
        assert_eq!(Sim::synced_text(60), "Synced 1h ago");
        assert_eq!(Sim::synced_text(23 * 60 + 59), "Synced 23h ago");
        assert_eq!(Sim::synced_text(2 * 24 * 60), "Synced 2d ago");
    }

    #[test]
    fn the_status_bar_follows_the_last_sync() {
        let mut sim = Sim::new();
        sim.set_last_sync_minutes_ago(Some(0));
        assert_eq!(crate::Store::status(&sim).sync_text, "Synced now");
        sim.set_last_sync_minutes_ago(Some(5));
        assert_eq!(crate::Store::status(&sim).sync_text, "Synced 5m ago");
        sim.set_last_sync_minutes_ago(None);
        assert_eq!(crate::Store::status(&sim).sync_text, "Not synced yet");
        // A sync that completes clears "never" and reads as now.
        sim.sync_began();
        assert_eq!(crate::Store::status(&sim).sync_text, "Syncing…");
        sim.sync_ended(vec![(true, "Jira (acli)".into(), "ok".into())]);
        assert_eq!(crate::Store::status(&sim).sync_text, "Synced now");
    }

    /// The engine's workspace list replaces the invented one; the open workspace's own data
    /// (its tickets) is not touched by the swap.
    #[test]
    fn the_saved_workspaces_can_be_replaced_with_the_real_ones() {
        let mut sim = Sim::new();
        let tickets = sim.tickets.len();
        sim.replace_workspaces(vec!["alpha".into(), "beta".into()], Some(&"beta".into()));
        assert_eq!(
            sim.current_workspace().map(|n| n.as_str()),
            Some("beta")
        );
        let vm = sim.workspaces_vm();
        assert_eq!(vm.current.as_ref().map(|n| n.as_str()), Some("beta"));
        // The open one counts as used just now, so it leads the list.
        assert_eq!(
            vm.items.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["beta", "alpha"]
        );
        assert_eq!(sim.tickets.len(), tickets, "the tickets are the caller's to load");

        sim.replace_workspaces(vec!["alpha".into(), "beta".into()], None);
        assert!(sim.current_workspace().is_none());
        assert_eq!(
            sim.workspaces_vm().items.len(),
            2,
            "closing keeps the list, it only forgets which is open"
        );
    }
}

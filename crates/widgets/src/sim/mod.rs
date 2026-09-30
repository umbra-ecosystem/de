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
mod store_impl;

pub use rules::{Rule, Sug};

use std::collections::BTreeMap;

use model::*;

use crate::vm::{Branch, MappingKey, RepoName, TicketKey, ToastKind};

pub struct Sim {
    pub(crate) ms: i64,
    /// The minute of the day (0..1440) that `ms == 0` is, so a clock reads as real time where it should.
    pub(crate) start_min: i64,
    pub(crate) seq: u64,
    pub(crate) run: u32,
    pub(crate) sync: SyncState,
    pub(crate) jira_ready: bool,
    pub(crate) gh_ready: bool,
    pub(crate) locks: BTreeMap<RepoName, Lock>,
    pub(crate) sims: Sims,
    pub(crate) ws: Workspace,
    pub(crate) wait_min: u32,
    /// The editable Jira mapping; a key that is absent is unset.
    pub(crate) mapping: BTreeMap<MappingKey, String>,
    pub(crate) tickets: Vec<Ticket>,
    pub(crate) audit: Vec<AuditEntry>,
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
        Self {
            ms: 0,
            start_min: START_MIN,
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
            mapping: [
                (MappingKey::ReviewJql, "project = PROJ AND status = \"In Review\""),
                (MappingKey::AccountId, "acct-0001"),
            ]
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect(),
            tickets: data::seed_tickets(),
            audit: Vec::new(),
            responses: Responses::new(),
            repos: repos(),
            toasts: Vec::new(),
        }
    }

    /// A simulation that starts from these tickets instead of the invented ones (the real store uses it for what
    /// the engine does not serve yet: PRs, review and shipping stay simulated).
    pub fn with_tickets(tickets: Vec<Ticket>) -> Self {
        Self {
            tickets,
            ..Self::new()
        }
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

    /// What minute of the day it is now (local time), for the times the audit log and waits show.
    pub fn set_start_minute_of_day(&mut self, minute: i64) {
        self.start_min = minute.rem_euclid(24 * 60);
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

    pub(crate) fn audit(
        &mut self,
        action: &str,
        ticket: Option<&TicketKey>,
        repo: Option<&RepoName>,
        outcome: AuditOutcome,
        details: &str,
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

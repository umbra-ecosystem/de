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

use crate::vm::{Branch, RepoName, TicketKey, ToastKind};

pub struct Sim {
    pub(crate) ms: i64,
    pub(crate) seq: u64,
    pub(crate) run: u32,
    pub(crate) sync: SyncState,
    pub(crate) jira_ready: bool,
    pub(crate) gh_ready: bool,
    pub(crate) locks: BTreeMap<RepoName, Lock>,
    pub(crate) sims: Sims,
    pub(crate) ws: Workspace,
    pub(crate) wait_min: u32,
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
            tickets: data::seed_tickets(),
            audit: Vec::new(),
            responses: Responses::new(),
            repos: repos(),
            toasts: Vec::new(),
        }
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
        let m = START_MIN + ms.div_euclid(MS_PER_MIN);
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

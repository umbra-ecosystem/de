//! How suggestions are ordered. This file is the one place to change it.
//!
//! A suggestion is compared with another by [`POLICY`], one [`Criterion`] at a time: the first criterion on
//! which they differ decides, and a later one only breaks a tie. So **the order of `POLICY` is the order of
//! importance.** To change how the list is ranked:
//!
//! - reorder, add or remove entries in [`POLICY`];
//! - change what a criterion means in [`Criterion::score`];
//! - change how important a rule is on its own in `Rule::band` (`rules.rs`), or which rules are blocking in
//!   [`Rule::is_blocking`].
//!
//! Scores are "bigger is earlier".

use super::model::Priority;
use super::rules::{Rule, Sug};

/// What matters, most important first.
pub const POLICY: &[Criterion] = &[
    Criterion::Blocking,
    Criterion::NotAwaitingPr,
    Criterion::Hotfix,
    Criterion::NeedsAttention,
    Criterion::Actionable,
    Criterion::Band,
    Criterion::TicketPriority,
    Criterion::QueuePosition,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Criterion {
    /// Something is broken or in the way (failed deploy, blocked integration, uat conflict, stale lock).
    Blocking,
    /// The ticket is not waiting for a pull request that has had no time to appear. Something with no PR yet
    /// (and a grace period still running, or not started) has nothing to act on, so it goes below everything
    /// that has, whatever its priority. Once the grace period has run out it is something to act on again.
    NotAwaitingPr,
    /// A hotfix you can act on now.
    Hotfix,
    /// Someone is waiting or something broke (`Rule::needs_attention`) and there is an action for it.
    NeedsAttention,
    /// There is something to do, as opposed to something to know. Information never outranks an action.
    Actionable,
    /// How important the rule is on its own (`Rule::band`).
    Band,
    /// The ticket's Jira priority, Highest first.
    TicketPriority,
    /// Where the ticket is in the claim queue, first first.
    QueuePosition,
}

impl Criterion {
    pub fn score(self, s: &Sug) -> i64 {
        match self {
            Criterion::Blocking => i64::from(s.rule.is_blocking()),
            Criterion::NotAwaitingPr => i64::from(!s.awaiting_pr),
            Criterion::Hotfix => i64::from(s.hotfix && !s.info && s.rule.is_boosted_by_hotfix()),
            Criterion::NeedsAttention => i64::from(!s.info && s.rule.needs_attention()),
            Criterion::Actionable => i64::from(!s.info),
            Criterion::Band => i64::from(s.rule.band()),
            Criterion::TicketPriority => match s.ticket_priority {
                Some(Priority::Highest) => 3,
                Some(Priority::High) => 2,
                Some(Priority::Medium) => 1,
                Some(Priority::Low) | None => 0,
            },
            Criterion::QueuePosition => s.queue.map_or(0, |i| -(i as i64)),
        }
    }
}

/// The sort key of one suggestion: its score on each criterion, in [`POLICY`] order.
pub fn key(s: &Sug) -> Vec<i64> {
    POLICY.iter().map(|c| c.score(s)).collect()
}

/// Most urgent first. Equal keys fall back to the suggestion's id so the order is stable.
pub fn compare(a: &Sug, b: &Sug) -> std::cmp::Ordering {
    key(b).cmp(&key(a)).then_with(|| a.id.cmp(&b.id))
}

impl Rule {
    /// Rules about something broken or in the way.
    pub fn is_blocking(self) -> bool {
        matches!(
            self,
            Rule::DeployFailed | Rule::IntegrationBlocked | Rule::UatConflict | Rule::StaleLock
        )
    }

    /// Whether a hotfix raises this rule. Parking the active ticket and tool notices are not hotfix work.
    pub fn is_boosted_by_hotfix(self) -> bool {
        !matches!(self, Rule::ParkActive | Rule::AdapterUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Sim;

    /// The order the demo gives today. If this changes on purpose, change it here.
    #[test]
    fn hotfix_first_then_actions_then_information() {
        let sim = Sim::default();
        let order: Vec<(String, bool)> = sim
            .suggest()
            .iter()
            .map(|s| (s.title.clone(), s.info))
            .collect();
        assert_eq!(order[0].0, "Claim PROJ-139", "the actionable hotfix leads");
        let first_info = order.iter().position(|(_, info)| *info).unwrap();
        assert!(
            order[..first_info].iter().all(|(_, info)| !info)
                && order[first_info..].iter().all(|(_, info)| *info),
            "information never outranks an action: {order:?}"
        );
    }

    fn claim(id: &str, p: Priority, hotfix: bool, awaiting_pr: bool) -> Sug {
        Sug {
            ticket_priority: Some(p),
            hotfix,
            awaiting_pr,
            ..Sim::base_sug(
                Rule::ClaimNew,
                id.to_string(),
                String::new(),
                String::new(),
                crate::vm::Intent::Noop,
                String::new(),
            )
        }
    }

    #[test]
    fn a_ticket_still_awaiting_its_pull_request_never_leads_however_urgent() {
        let low_with_pr = claim("a-low", Priority::Low, false, false);
        let highest_waiting = claim("b-highest", Priority::Highest, false, true);
        let hotfix_waiting = claim("c-hotfix", Priority::Highest, true, true);
        assert_eq!(
            compare(&low_with_pr, &highest_waiting),
            std::cmp::Ordering::Less,
            "a low priority ticket with a PR beats a Highest one without"
        );
        assert_eq!(
            compare(&low_with_pr, &hotfix_waiting),
            std::cmp::Ordering::Less,
            "and beats a hotfix that has no PR yet"
        );
        // Among the waiting ones the usual order still holds.
        assert_eq!(
            compare(&hotfix_waiting, &highest_waiting),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn once_the_grace_period_has_run_out_the_ticket_ranks_by_its_priority_again() {
        let expired_highest = claim("a", Priority::Highest, false, false);
        let low_with_pr = claim("b", Priority::Low, false, false);
        assert_eq!(compare(&expired_highest, &low_with_pr), std::cmp::Ordering::Less);
    }

    #[test]
    fn a_blocking_problem_still_outranks_a_ticket_awaiting_its_pr() {
        let failed = Sug {
            awaiting_pr: true,
            ..claim("deploy", Priority::Low, false, true)
        };
        let failed = Sug { rule: Rule::DeployFailed, ..failed };
        let fine = claim("fine", Priority::Highest, false, false);
        assert_eq!(compare(&failed, &fine), std::cmp::Ordering::Less);
    }

    /// In the demo world: no suggestion about a ticket that awaits its PR comes before one that does not (blocking
    /// problems aside), and the flag is exactly "no PR and the grace period is not over".
    #[test]
    fn in_the_demo_nothing_awaiting_a_pr_precedes_something_that_is_not() {
        let mut sim = Sim::default();
        // Nothing in hand, so every ticket in the pool can be claimed and the claim queue is the whole list.
        for t in &mut sim.tickets {
            if matches!(
                t.local(),
                Some(crate::sim::model::Local::Claimed | crate::sim::model::Local::Reviewing | crate::sim::model::Local::Parked)
            ) {
                t.stage = crate::sim::model::Stage::Unclaimed;
            }
        }
        let all = sim.suggest();
        assert!(all.iter().any(|s| s.awaiting_pr), "the demo has a ticket without a PR");
        assert!(all.iter().any(|s| !s.awaiting_pr && s.ticket.is_some()));
        let first_waiting = all
            .iter()
            .position(|s| s.awaiting_pr)
            .unwrap();
        for s in &all[first_waiting..] {
            assert!(
                s.awaiting_pr || s.rule.is_blocking(),
                "{} ({:?}) follows something awaiting a PR",
                s.title,
                s.rule
            );
        }
        for s in &all {
            if let Some(t) = s.ticket.as_ref().and_then(|k| sim.tk(k)) {
                assert_eq!(
                    s.awaiting_pr,
                    sim.pr_gap(t).is_some() && !sim.pr_wait_expired(t),
                    "{}",
                    s.title
                );
            }
        }
    }

    #[test]
    fn a_blocking_problem_outranks_a_hotfix_and_ties_go_to_ticket_priority() {
        let mk = |rule: Rule, hotfix: bool, info: bool, p: Option<Priority>| Sug {
            ticket_priority: p,
            hotfix,
            info,
            ..Sim::base_sug(
                rule,
                format!("{rule:?}"),
                String::new(),
                String::new(),
                crate::vm::Intent::Noop,
                String::new(),
            )
        };
        let failed = mk(Rule::DeployFailed, false, false, Some(Priority::Low));
        let hot = mk(Rule::ClaimNew, true, false, Some(Priority::Highest));
        let routine = mk(Rule::StartReview, false, false, Some(Priority::Low));
        assert_eq!(
            compare(&failed, &hot),
            std::cmp::Ordering::Less,
            "blocking beats hotfix"
        );
        assert_eq!(
            compare(&hot, &routine),
            std::cmp::Ordering::Less,
            "hotfix beats routine"
        );
        let urgent = mk(Rule::StartReview, false, false, Some(Priority::Highest));
        assert_eq!(
            compare(&urgent, &routine),
            std::cmp::Ordering::Less,
            "same rule: higher priority first"
        );
    }
}

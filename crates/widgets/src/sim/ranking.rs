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
    Criterion::Hotfix,
    Criterion::Actionable,
    Criterion::Band,
    Criterion::TicketPriority,
    Criterion::QueuePosition,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Criterion {
    /// Something is broken or in the way (failed deploy, blocked integration, uat conflict, stale lock).
    Blocking,
    /// A hotfix you can act on now.
    Hotfix,
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
            Criterion::Hotfix => i64::from(s.hotfix && !s.info && s.rule.is_boosted_by_hotfix()),
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

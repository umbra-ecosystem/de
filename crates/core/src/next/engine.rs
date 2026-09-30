//! [`suggest`]: run the rules over a snapshot and turn their raw output into the ranked list.
//!
//! Pure: no I/O, no clock (`now` is passed in), no writer of any kind. The steps after the
//! rules are, in order:
//!
//! 1. **hotfix**: a hotfix ticket's suggestions are flagged (`facts.hotfix`, a "Hotfix:"
//!    prefix) and raised by [`prio::HOTFIX_BOOST`];
//! 2. **unavailable adapters**: a gateway suggestion whose writer is not available is
//!    replaced by an informational one saying so, never silently dropped;
//! 3. **de-duplication**: two suggestions with the same id, or the same ticket and the same
//!    action, collapse into the higher-priority one;
//! 4. **ordering**: total and stable, see [`sort`].

use serde_json::{Value, json};

use super::{
    model::{ExecutionLevel, RuleId, SuggestedAction, Suggestion, prio},
    responses::apply_responses,
    rules::all_rules,
    snapshot::{Availability, Snapshot, jira_priority_rank},
};
use crate::{gateway::Action, store::suggestion_responses::SuggestionResponse};
use std::collections::BTreeMap;

/// The ranked suggestions for `snapshot`, most urgent first. See the module docs.
pub fn suggest(snapshot: &Snapshot, now: i64) -> Vec<Suggestion> {
    let mut all = all_rules(snapshot, now);
    flag_hotfix(snapshot, &mut all);
    replace_unavailable(snapshot, &mut all);
    let mut all = dedupe(all);
    sort(snapshot, &mut all);
    all
}

/// [`suggest`], minus what the author dismissed, snoozed or finished (see
/// [`apply_responses`]).
pub fn next_actions(
    snapshot: &Snapshot,
    responses: &BTreeMap<String, SuggestionResponse>,
    now: i64,
) -> Vec<Suggestion> {
    apply_responses(suggest(snapshot, now), responses, now)
}

fn flag_hotfix(s: &Snapshot, all: &mut [Suggestion]) {
    for sug in all.iter_mut() {
        let Some(key) = &sug.ticket else { continue };
        let hotfix = s.ticket(key).is_some_and(|t| t.is_hotfix());
        if !hotfix
            || sug.rule == RuleId::ParkActive
            || sug.priority.0 < prio::QUEUE_FLOOR
            || sug.priority.0 >= prio::AUTOMATIC
        {
            continue;
        }
        sug.priority.0 += prio::HOTFIX_BOOST;
        if let Value::Object(map) = &mut sug.facts {
            map.insert("hotfix".into(), json!(true));
        }
        sug.reason = format!("Hotfix: {}", sug.reason);
    }
}

/// Which adapter a gateway action needs, and what it would have done.
fn gateway_need(action: &Action) -> Option<(&'static str, String)> {
    match action {
        Action::PushUat(_) => None,
        Action::PostJiraComment { ticket, .. } => {
            Some(("jira", format!("post the deploy comment on {ticket}")))
        }
        Action::TransitionJira { ticket, to_status } => {
            Some(("jira", format!("move {ticket} to {to_status}")))
        }
        Action::ApprovePr { repo, pr } => Some(("bitbucket", format!("approve {repo} #{pr}"))),
        Action::RerunPipeline { repo, run_id } => {
            Some(("bitbucket", format!("re-run pipeline {run_id} of {repo}")))
        }
        Action::PostPrComment { repo, pr, .. } => {
            Some(("bitbucket", format!("comment on {repo} #{pr}")))
        }
        Action::RequestChanges { repo, pr, .. } => {
            Some(("bitbucket", format!("request changes on {repo} #{pr}")))
        }
        Action::TriggerPipeline { repo, .. } => {
            Some(("bitbucket", format!("trigger a pipeline of {repo}")))
        }
    }
}

fn replace_unavailable(s: &Snapshot, all: &mut [Suggestion]) {
    for sug in all.iter_mut() {
        let SuggestedAction::Gateway(action) = &sug.action else {
            continue;
        };
        let Some((adapter, blocked)) = gateway_need(action) else {
            continue;
        };
        let availability = if adapter == "jira" {
            &s.writers.jira
        } else {
            &s.writers.code_host
        };
        let Availability::Unavailable(detail) = availability else {
            continue;
        };
        let original = sug.rule;
        let priority = sug.priority.0.min(prio::ADAPTER_UNAVAILABLE);
        *sug = Suggestion::new(
            sug.ticket.as_ref(),
            RuleId::AdapterUnavailable,
            &format!("{adapter}:{original}"),
            SuggestedAction::AdapterUnavailable {
                adapter: adapter.into(),
                detail: detail.clone(),
                blocked: blocked.clone(),
            },
            format!("Cannot {blocked}: the {adapter} adapter is not available ({detail})"),
            json!({ "adapter": adapter, "detail": detail, "original_rule": original.as_str() }),
            priority,
        );
        debug_assert_eq!(sug.level, ExecutionLevel::Automatic);
    }
}

fn dedupe(all: Vec<Suggestion>) -> Vec<Suggestion> {
    let mut kept: Vec<Suggestion> = Vec::new();
    for candidate in all {
        let same = kept.iter().position(|k| {
            k.id == candidate.id || (k.ticket == candidate.ticket && k.action == candidate.action)
        });
        match same {
            Some(i) if candidate.priority > kept[i].priority => kept[i] = candidate,
            Some(_) => {}
            None => kept.push(candidate),
        }
    }
    kept
}

/// Sorts into the final, total order:
///
/// 1. priority, descending;
/// 2. the ticket's place: tracked tickets by manual order, then untracked ones by Jira
///    priority rank; suggestions about no ticket last;
/// 3. the ticket key;
/// 4. the rule (declaration order of [`RuleId`]);
/// 5. the id.
pub fn sort(s: &Snapshot, all: &mut [Suggestion]) {
    let place = |sug: &Suggestion| -> (u8, i64) {
        match sug.ticket.as_ref().and_then(|k| s.ticket(k)) {
            Some(t) => match &t.tracking {
                Some(tr) => (0, tr.manual_order),
                None => (1, i64::from(jira_priority_rank(t.jira_priority.as_deref()))),
            },
            None => (2, 0),
        }
    };
    all.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| place(a).cmp(&place(b)))
            .then_with(|| a.ticket.cmp(&b.ticket))
            .then_with(|| a.rule.cmp(&b.rule))
            .then_with(|| a.id.cmp(&b.id))
    });
}

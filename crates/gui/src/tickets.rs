//! The synced tickets as the widgets model them.
//!
//! `cache.db` (the Jira mirror) joined with `state.db` (local tracking) becomes [`Ticket`]s. Jira's status
//! names are matched to the prototype's four through the configured names; any other status keeps Jira's own
//! spelling. The mapping is pure so it can be tested against temporary databases.

use std::collections::HashSet;
use std::sync::Mutex;

use de_core::config::Config;
use de_core::domain::LocalStatus;
use de_core::store::{Store, jira_cache, jira_comments};
use de_widgets::sim::model::{
    Activation, Comment, Held, JiraStatus, Priority, Stage, Ticket,
};
use de_widgets::vm::{Block, Phase};

/// Every ticket the engine knows: the cached pool plus anything tracked by hand.
pub fn load(state: &Store, cache: &Store, config: &Config) -> eyre::Result<Vec<Ticket>> {
    let mut out = Vec::new();
    for view in jira_cache::list_views(state, cache)? {
        let Some(jira) = view.jira else {
            // Tracked but never fetched: there is nothing to show yet except the key.
            if let Some(tracking) = view.tracking {
                let mut t = Ticket::blank(view.key.as_str(), view.key.as_str());
                t.stage = stage_of(tracking.status);
                out.push(t);
            }
            continue;
        };
        let mut t = Ticket::blank(jira.key.as_str(), &jira.title);
        let (status, name) = status_of(&jira.jira_status, config);
        t.jira = status;
        t.jira_name = name;
        t.priority = priority_of(jira.priority.as_deref());
        t.assignee = leak(jira.assignee.as_deref().unwrap_or(""));
        t.updated = leak(&format_time(jira.fetched_at));
        t.comments = jira_comments::list_for_ticket(cache, &jira.key)?
            .into_iter()
            .enumerate()
            .map(|(i, c)| Comment {
                n: i as u32 + 1,
                who: c.author_name,
                at: format_time(c.created_at),
                body: vec![Block::Para(c.body_text)],
            })
            .collect();
        if let Some(tracking) = view.tracking {
            t.stage = stage_of(tracking.status);
        }
        out.push(t);
    }
    Ok(out)
}

/// Jira's status name against the configured workflow names (case-insensitive).
pub fn status_of(name: &str, config: &Config) -> (JiraStatus, Option<String>) {
    let s = config.jira_statuses();
    let is = |other: &str| other.eq_ignore_ascii_case(name);
    let status = if is(s.review_name()) {
        JiraStatus::InReview
    } else if is(s.alpha_testing_name()) {
        JiraStatus::AlphaTesting
    } else if is(s.returned_name()) {
        JiraStatus::Returned
    } else if s.done_names().into_iter().any(is) {
        JiraStatus::Done
    } else {
        return (JiraStatus::Other, Some(name.to_string()));
    };
    (status, None)
}

pub fn priority_of(name: Option<&str>) -> Priority {
    match name.map(str::to_ascii_lowercase).as_deref() {
        Some("highest" | "blocker" | "critical") => Priority::Highest,
        Some("high" | "major") => Priority::High,
        Some("low" | "lowest" | "minor" | "trivial") => Priority::Low,
        _ => Priority::Medium,
    }
}

/// The widgets' stage for a local status. Records of an activation are not read yet, so an active ticket
/// carries an empty one.
pub fn stage_of(status: LocalStatus) -> Stage {
    let held = Held::default();
    let in_hand = |phase| Stage::InHand { phase, held: Held::default() };
    match status {
        LocalStatus::Claimed => in_hand(Phase::Claimed),
        LocalStatus::Reviewing => in_hand(Phase::Reviewing),
        LocalStatus::Parked => in_hand(Phase::Parked),
        LocalStatus::Active => Stage::Active {
            held,
            act: Activation {
                started_ms: 0,
                records: Vec::new(),
                overlay: None,
            },
            prep: None,
        },
        LocalStatus::Integrated => Stage::Integrated { held },
        LocalStatus::Done => Stage::Done { held },
    }
}

/// Text that lives as long as the app: the widgets' ticket fields are `&'static str`. Interned, so repeated
/// syncs do not grow memory.
pub fn leak(text: &str) -> &'static str {
    static SEEN: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut guard = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    if let Some(found) = set.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    set.insert(leaked);
    leaked
}

/// `2026-10-01 09:40` (UTC) from unix seconds.
pub fn format_time(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_core::providers::RemoteTicket;
    use de_core::store::{Kind, tickets as tracking};

    fn remote(key: &str, status: &str, priority: &str) -> RemoteTicket {
        RemoteTicket {
            key: key.parse().unwrap(),
            title: format!("Title of {key}"),
            status: status.into(),
            priority: Some(priority.into()),
            assignee: Some("Alex Example".into()),
            url: None,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
    fn formats_unix_time() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
    }

    #[test]
    fn statuses_use_configured_names_and_keep_unknown_ones() {
        let config = Config::default();
        assert_eq!(status_of("in review", &config), (JiraStatus::InReview, None));
        assert_eq!(status_of("Returned", &config).0, JiraStatus::Returned);
        assert_eq!(status_of("Done", &config).0, JiraStatus::Done);
        assert_eq!(
            status_of("Blocked", &config),
            (JiraStatus::Other, Some("Blocked".into()))
        );
    }

    #[test]
    fn joins_the_mirror_with_local_tracking() {
        let state = Store::open_in_memory(Kind::State).unwrap();
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-1", "In Review", "High"), 10).unwrap();
        jira_cache::upsert_remote(&cache, &remote("PROJ-2", "Returned", "Low"), 10).unwrap();
        tracking::claim(&state, &"PROJ-2".parse().unwrap(), 20).unwrap();

        let mut all = load(&state, &cache, &Config::default()).unwrap();
        all.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].title, "Title of PROJ-1");
        assert_eq!(all[0].priority, Priority::High);
        assert_eq!(all[0].assignee, "Alex Example");
        assert!(all[0].stage.local().is_none(), "unclaimed stays unclaimed");
        assert_eq!(all[1].jira, JiraStatus::Returned);
        assert_eq!(all[1].stage.name(), "claimed");
    }
}

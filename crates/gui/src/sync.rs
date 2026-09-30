//! A read-only Jira sync through the engine, and its result in the terms the status bar shows.
//!
//! [`sync_now`] takes the provider as a value so tests can pass a fake. The app builds the real `acli`
//! adapter with [`de_core::providers::Providers`]. Nothing here can write to Jira: the sync API accepts only
//! read traits.

use de_core::config::Config;
use de_core::providers::{ProviderError, ProviderErrorKind, Providers, TicketProvider};
use de_core::store::{Kind, Store};
use de_core::sync::{
    DEFAULT_MIN_INTERVAL, SourceOutcome, SourceReport, SyncContext, SyncReport, SyncSource,
    sync_all,
};
use de_widgets::sim::model::Ticket;

use crate::tickets;

/// What one sync produced.
pub struct SyncDone {
    /// `(ok, source, text)` per source, as [`de_widgets::sim::Sim::sync_ended`] takes it.
    pub report: Vec<(bool, String, String)>,
    /// The tickets as they are in the cache after the sync (also after a failed one: the cache is kept).
    pub tickets: Result<Vec<Ticket>, String>,
    /// Jira could not be used because the login is missing or the tool is not installed.
    pub jira_unavailable: bool,
    /// Something was read in full or removed, or something failed: worth a line in the audit log when the sync
    /// was automatic.
    pub changed: bool,
}

/// Sync Jira into `cache`, then read every ticket back.
pub fn sync_now(
    state: &Store,
    cache: &Store,
    config: &Config,
    jira: Result<&dyn TicketProvider, &ProviderError>,
    now: i64,
) -> SyncDone {
    let ctx = SyncContext {
        state,
        cache,
        config,
        now,
        force: true,
        min_interval: DEFAULT_MIN_INTERVAL,
        full: false,
    };
    // No code host yet (GitHub is not wired): only Jira is asked for.
    let no_host = ProviderError::not_installed("code host", "not connected yet");
    let report = sync_all(&ctx, jira, Err(&no_host), &[], &[], Some(SyncSource::Jira));
    let jira_unavailable = report.sources.iter().any(|s| {
        matches!(
            &s.outcome,
            SourceOutcome::Failed(p)
                if matches!(p.kind, ProviderErrorKind::NotAuthenticated | ProviderErrorKind::NotInstalled)
        )
    });
    let changed = !report.is_ok()
        || report
            .sources
            .iter()
            .any(|s| s.counts.details > 0 || s.counts.removed > 0);
    SyncDone {
        report: describe(&report),
        tickets: tickets::load(state, cache, config).map_err(|e| format!("{e:#}")),
        jira_unavailable,
        changed,
    }
}

/// The whole thing the app's background thread does: open the databases, build `acli`, sync.
pub fn sync_real(now: i64) -> SyncDone {
    let opened = Config::load()
        .and_then(|c| Ok((c, Store::open_default(Kind::State)?, Store::open_default(Kind::Cache)?)));
    match opened {
        Ok((config, state, cache)) => {
            // Kept until the sync is done; dropping it ends the run.
            let _run_log = de_core::synclog::begin_default(now, &config);
            let providers = Providers::from_config(&config);
            sync_now(&state, &cache, &config, providers.jira.as_deref(), now)
        }
        Err(e) => SyncDone {
            report: vec![(false, "Jira (acli)".into(), format!("local error: {e:#}"))],
            tickets: Err(format!("{e:#}")),
            jira_unavailable: false,
            changed: true,
        },
    }
}

/// Whole minutes since Jira last synced completely, from the cache; `None` when it never has.
pub fn last_sync_minutes_ago(now: i64) -> Option<i64> {
    let cache = Store::open_default(Kind::Cache).ok()?;
    let state = de_core::store::sync_state::get(&cache, de_core::store::sync_state::JIRA).ok()??;
    Some(((now - state.last_ok_at?).max(0)) / 60)
}

/// Read the cached tickets without syncing (what the app shows at start, offline or not).
pub fn load_cached() -> eyre::Result<Vec<Ticket>> {
    let config = Config::load()?;
    let state = Store::open_default(Kind::State)?;
    let cache = Store::open_default(Kind::Cache)?;
    tickets::load(&state, &cache, &config)
}

fn describe(report: &SyncReport) -> Vec<(bool, String, String)> {
    report
        .sources
        .iter()
        .map(|s| {
            (
                matches!(s.outcome, SourceOutcome::Synced | SourceOutcome::Skipped { .. }),
                "Jira (acli)".to_string(),
                describe_source(s),
            )
        })
        .collect()
}

fn describe_source(s: &SourceReport) -> String {
    let c = &s.counts;
    let mut text = match &s.outcome {
        SourceOutcome::Synced => format!(
            "{} ticket{}, {} comment{}{}",
            c.tickets,
            plural(c.tickets),
            c.ticket_comments,
            plural(c.ticket_comments),
            if c.removed > 0 {
                format!(", {} stale removed", c.removed)
            } else {
                String::new()
            }
        ),
        SourceOutcome::Partial(problems) => format!(
            "partial: {}",
            problems
                .first()
                .map(|p| p.message.clone())
                .unwrap_or_default()
        ),
        SourceOutcome::Failed(p) => format!("{}, cache kept: {}", p.kind, p.message),
        SourceOutcome::Skipped { .. } => "fresh".to_string(),
        SourceOutcome::NotConfigured(reason) => format!("not configured: {reason}"),
    };
    for note in &s.notes {
        text.push_str(&format!("; {note}"));
    }
    text
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_core::providers::fake::FakeJira;
    use de_core::providers::fake::build::{comment, key, ticket};
    use de_core::store::{jira_cache, tickets as tracking};

    const POOL: &str = "project = PROJ AND status = 'In Review'";

    fn config() -> Config {
        Config::parse(&format!(
            "[jira]\nsite = \"x\"\naccount_id = \"me\"\nreview_jql = \"{POOL}\"\n"
        ))
        .unwrap()
    }

    fn stores() -> (Store, Store) {
        (
            Store::open_in_memory(Kind::State).unwrap(),
            Store::open_in_memory(Kind::Cache).unwrap(),
        )
    }

    #[test]
    fn a_sync_brings_the_pool_and_comments_into_the_widget_tickets() {
        let (state, cache) = stores();
        let jira = FakeJira::new();
        jira.on_search(POOL, vec![ticket("PROJ-1", "In Review"), ticket("PROJ-2", "In Review")]);
        jira.on_search("status = \"Returned\"", vec![ticket("PROJ-3", "Returned")]);
        jira.set_comments(&key("PROJ-3"), vec![comment("PROJ-3", "c1", &["me"])]);

        let done = sync_now(&state, &cache, &config(), Ok(&jira), 1_000);

        assert!(done.report.iter().all(|r| r.0), "{:?}", done.report);
        assert_eq!(done.report[0].2, "3 tickets, 1 comment");
        let mut tickets = done.tickets.unwrap();
        tickets.sort_by(|a, b| a.key.cmp(&b.key));
        let keys: Vec<&str> = tickets.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(keys, ["PROJ-1", "PROJ-2", "PROJ-3"]);
        assert_eq!(tickets[2].comments.len(), 1);
        assert_eq!(tickets[2].comments[0].who, "Someone");
        assert!(!jira.log().calls().iter().any(|c| c.is_write()), "sync must not write");
    }

    #[test]
    fn a_ticket_that_left_the_pool_disappears_only_after_a_complete_sync() {
        let (state, cache) = stores();
        let jira = FakeJira::new();
        jira.on_search(POOL, vec![ticket("PROJ-1", "In Review"), ticket("PROJ-2", "In Review")]);
        sync_now(&state, &cache, &config(), Ok(&jira), 1_000);

        jira.on_search(POOL, vec![ticket("PROJ-1", "In Review")]);
        let done = sync_now(&state, &cache, &config(), Ok(&jira), 2_000);
        assert_eq!(done.tickets.unwrap().len(), 1);
        assert!(done.report[0].2.contains("1 stale removed"), "{:?}", done.report);
    }

    #[test]
    fn a_claimed_ticket_keeps_its_local_stage_and_survives_leaving_the_pool() {
        let (state, cache) = stores();
        let jira = FakeJira::new();
        jira.on_search(POOL, vec![ticket("PROJ-1", "In Review")]);
        sync_now(&state, &cache, &config(), Ok(&jira), 1_000);
        tracking::claim(&state, &key("PROJ-1"), 1_001).unwrap();

        // Jira moved it on; tracked tickets are still fetched by key.
        jira.on_search(POOL, vec![]);
        jira.add_ticket(ticket("PROJ-1", "Alpha Testing"));
        let done = sync_now(&state, &cache, &config(), Ok(&jira), 2_000);

        let tickets = done.tickets.unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].stage.name(), "claimed");
        assert_eq!(tickets[0].jira.label(), "Alpha Testing");
    }

    #[test]
    fn offline_keeps_the_cache_and_says_so() {
        let (state, cache) = stores();
        let jira = FakeJira::new();
        jira.on_search(POOL, vec![ticket("PROJ-1", "In Review")]);
        sync_now(&state, &cache, &config(), Ok(&jira), 1_000);

        jira.set_offline();
        let done = sync_now(&state, &cache, &config(), Ok(&jira), 2_000);

        assert!(!done.report[0].0);
        assert!(done.report[0].2.contains("cache kept"), "{:?}", done.report);
        assert!(!done.jira_unavailable, "offline is not a missing login");
        assert_eq!(done.tickets.unwrap().len(), 1, "the cached ticket is still shown");
        assert_eq!(jira_cache::list(&cache).unwrap().len(), 1);
    }

    #[test]
    fn a_missing_login_is_reported_as_unavailable() {
        let (state, cache) = stores();
        let jira = FakeJira::new();
        jira.set_unauthenticated();
        let done = sync_now(&state, &cache, &config(), Ok(&jira), 1_000);
        assert!(!done.report[0].0);
        assert!(done.jira_unavailable);
    }

    #[test]
    fn no_jira_section_still_syncs_with_the_default_query() {
        let (state, cache) = stores();
        let jira = FakeJira::new();
        jira.on_search(
            "status = \"In Review\" AND updated >= -30d ORDER BY updated DESC",
            vec![ticket("PROJ-1", "In Review")],
        );
        let done = sync_now(&state, &cache, &Config::default(), Ok(&jira), 1_000);
        assert!(done.report[0].0, "{:?}", done.report);
        assert_eq!(done.tickets.unwrap().len(), 1);
    }

    /// Runs the real `acli`, read-only, into throwaway databases:
    /// `cargo test -p de-gui -- --ignored live_`. Prints counts only, never ticket text.
    #[test]
    #[ignore = "runs the real installed acli (read-only)"]
    fn live_sync_reads_real_tickets_into_the_widget_model() {
        let (state, cache) = stores();
        let config = Config::parse(
            "[jira]\nsite = \"live\"\nreview_jql = \"assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC\"\n",
        )
        .unwrap();
        let providers = Providers::from_config(&config);
        let done = sync_now(&state, &cache, &config, providers.jira.as_deref(), 1_700_000_000);

        eprintln!("report: {:?}", done.report.iter().map(|r| (r.0, r.2.len())).collect::<Vec<_>>());
        assert!(done.report.iter().all(|r| r.0), "{:?}", done.report);
        let tickets = done.tickets.unwrap();
        eprintln!("tickets: {}", tickets.len());
        assert_eq!(tickets.len(), jira_cache::list(&cache).unwrap().len());
        for t in &tickets {
            assert!(!t.title.is_empty() && !t.key.is_empty());
        }
    }
}

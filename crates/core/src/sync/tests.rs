//! Sync engine tests: the fakes for the providers, real SQLite for the stores.

use super::*;
use crate::config::Config;
use crate::domain::TicketKey;
use crate::providers::fake::build::{comment, key, pr, run, ticket};
use crate::providers::fake::{FakeBitbucket, FakeJira};
use crate::providers::{
    TicketDetail, PipelineState, PipelineStep, PrState, ProviderError, ProviderErrorKind, RemoteTicket,
};
use crate::store::{
    Kind, Store, jira_cache, jira_comments, jira_details, links, pipelines, prs, sync_state, tickets, uat_merges,
};

const POOL_JQL: &str = "project = PROJ AND status = 'In Review'";
const RETURNED_JQL: &str = "status = \"Returned\"";

struct World {
    state: Store,
    cache: Store,
    config: Config,
    jira: FakeJira,
    bb: FakeBitbucket,
}

impl World {
    fn new() -> Self {
        let config = Config::parse(&format!(
            "[jira]\nsite = \"x\"\naccount_id = \"me\"\nreview_jql = \"{POOL_JQL}\"\n[bitbucket]\nworkspace = \"acme\"\n"
        ))
        .unwrap();
        Self {
            state: Store::open_in_memory(Kind::State).unwrap(),
            cache: Store::open_in_memory(Kind::Cache).unwrap(),
            config,
            jira: FakeJira::new(),
            bb: FakeBitbucket::new(),
        }
    }

    fn ctx(&self, now: i64, force: bool) -> SyncContext<'_> {
        SyncContext {
            state: &self.state,
            cache: &self.cache,
            config: &self.config,
            now,
            force,
            min_interval: 60,
        }
    }

    fn track(&self, k: &str, project: &str, branch: &str) {
        tickets::claim(&self.state, &key(k), 1).unwrap();
        links::add_manual(&self.state, &key(k), project, Some(branch)).unwrap();
    }

    fn cached_keys(&self) -> Vec<String> {
        jira_cache::list(&self.cache)
            .unwrap()
            .into_iter()
            .map(|t| t.key.to_string())
            .collect()
    }
}

fn hosted(project: &str, repo: &str) -> HostedRepo {
    HostedRepo {
        project: project.into(),
        repo: repo.into(),
        dir: format!("/code/{project}").into(),
        branches: Default::default(),
        deploy_environment: None,
    }
}

fn outcome_kind(r: &SourceReport) -> Option<ProviderErrorKind> {
    r.problem().map(|p| p.kind)
}

// ------------------------------------------------------------------ Jira

#[test]
fn jira_sync_refreshes_pool_returned_tracked_and_comments() {
    let w = World::new();
    w.jira.on_search(
        POOL_JQL,
        vec![ticket("PROJ-1", "In Review"), ticket("PROJ-2", "In Review")],
    );
    w.jira
        .on_search(RETURNED_JQL, vec![ticket("PROJ-3", "Returned")]);
    // Tracked, but no longer in the pool: still refreshed by key.
    w.track("PROJ-4", "web", "feature/PROJ-4");
    w.jira.add_ticket(ticket("PROJ-4", "Alpha Testing"));
    w.jira
        .set_comments(&key("PROJ-3"), vec![comment("PROJ-3", "c1", &["me"])]);
    w.jira
        .set_comments(&key("PROJ-4"), vec![comment("PROJ-4", "c2", &[])]);
    // Pool tickets get their comments and detail too (one `view` each).
    w.jira.set_detail(
        &key("PROJ-1"),
        TicketDetail {
            reporter: Some("Riley Reporter".into()),
            description: "Do the thing".into(),
            ..TicketDetail::default()
        },
    );
    w.jira
        .set_comments(&key("PROJ-1"), vec![comment("PROJ-1", "c9", &["me"])]);

    let report = sync_jira(&w.ctx(1000, false), &w.jira);
    assert_eq!(report.outcome, SourceOutcome::Synced, "{report:?}");
    assert_eq!(w.cached_keys(), ["PROJ-1", "PROJ-2", "PROJ-3", "PROJ-4"]);
    assert_eq!(
        jira_cache::get(&w.cache, &key("PROJ-4"))
            .unwrap()
            .unwrap()
            .jira_status,
        "Alpha Testing"
    );

    let mut mentions: Vec<String> = jira_comments::mentioning(&w.cache, "me")
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    mentions.sort();
    assert_eq!(mentions, ["c1", "c9"]);
    assert_eq!(
        jira_comments::list_for_ticket(&w.cache, &key("PROJ-1"))
            .unwrap()
            .len(),
        1
    );
    let detail = jira_details::get(&w.cache, &key("PROJ-1")).unwrap().unwrap();
    assert_eq!(detail.reporter.as_deref(), Some("Riley Reporter"));
    assert_eq!(detail.description, "Do the thing");
    assert!(jira_details::get(&w.cache, &key("PROJ-2")).unwrap().is_none(), "the fake gave it none");
    assert_eq!(w.jira.log().args_of("get"), ["PROJ-4"]);
    assert_eq!(
        sync_state::get(&w.cache, "jira")
            .unwrap()
            .unwrap()
            .last_ok_at,
        Some(1000)
    );
    assert!(w.jira.log().writes().is_empty());
}

#[test]
fn tickets_that_left_the_pool_are_removed_unless_tracked() {
    let w = World::new();
    w.jira.on_search(
        POOL_JQL,
        vec![ticket("PROJ-1", "In Review"), ticket("PROJ-2", "In Review")],
    );
    w.track("PROJ-2", "web", "feature/PROJ-2");
    sync_jira(&w.ctx(1000, false), &w.jira);
    assert_eq!(w.cached_keys(), ["PROJ-1", "PROJ-2"]);

    // Both leave the pool; only the tracked one survives (and is refreshed by key).
    w.jira.on_search(POOL_JQL, vec![]);
    w.jira.add_ticket(ticket("PROJ-2", "Alpha Testing"));
    let report = sync_jira(&w.ctx(2000, false), &w.jira);
    assert_eq!(report.outcome, SourceOutcome::Synced);
    assert_eq!(w.cached_keys(), ["PROJ-2"]);
    assert!(report.counts.removed >= 1);
}

#[test]
fn a_failed_pool_search_removes_nothing() {
    let w = World::new();
    w.jira
        .on_search(POOL_JQL, vec![ticket("PROJ-1", "In Review")]);
    sync_jira(&w.ctx(1000, false), &w.jira);

    // A bad-JQL style failure: not environmental, so the rest runs, but nothing is deleted.
    w.jira.fail_search(ProviderError::Command {
        tool: "acli".into(),
        status: Some(1),
        stderr: "bad jql".into(),
    });
    let report = sync_jira(&w.ctx(2000, false), &w.jira);
    assert!(
        matches!(report.outcome, SourceOutcome::Partial(_)),
        "{report:?}"
    );
    assert_eq!(outcome_kind(&report), Some(ProviderErrorKind::Command));
    assert_eq!(w.cached_keys(), ["PROJ-1"]);
    let state = sync_state::get(&w.cache, "jira").unwrap().unwrap();
    assert_eq!(state.last_ok_at, Some(1000));
    assert!(state.last_error.unwrap().contains("bad jql"));
}

#[test]
fn offline_and_unauthenticated_keep_the_cache_and_say_why() {
    let w = World::new();
    w.jira
        .on_search(POOL_JQL, vec![ticket("PROJ-1", "In Review")]);
    w.track("PROJ-9", "web", "b");
    w.jira
        .set_comments(&key("PROJ-9"), vec![comment("PROJ-9", "c1", &["me"])]);
    w.jira.add_ticket(ticket("PROJ-9", "In Review"));
    sync_jira(&w.ctx(1000, false), &w.jira);

    w.jira.set_offline();
    let report = sync_jira(&w.ctx(2000, false), &w.jira);
    assert!(matches!(report.outcome, SourceOutcome::Failed(_)));
    assert_eq!(outcome_kind(&report), Some(ProviderErrorKind::Network));
    assert_eq!(w.cached_keys(), ["PROJ-1", "PROJ-9"]);
    assert_eq!(jira_comments::mentioning(&w.cache, "me").unwrap().len(), 1);
    let state = sync_state::get(&w.cache, "jira").unwrap().unwrap();
    assert_eq!(
        (state.last_ok_at, state.last_attempt_at),
        (Some(1000), 2000)
    );
    // It stopped at the first environmental error instead of hammering the provider.
    assert_eq!(w.jira.log().count("search"), 3);

    w.jira.set_failure(None);
    w.jira.set_unauthenticated();
    let report = sync_jira(&w.ctx(3000, false), &w.jira);
    assert_eq!(
        outcome_kind(&report),
        Some(ProviderErrorKind::NotAuthenticated)
    );
    assert_eq!(w.cached_keys(), ["PROJ-1", "PROJ-9"]);
}

#[test]
fn a_ticket_deleted_in_jira_is_a_problem_but_does_not_stop_the_rest_or_lose_the_cache() {
    let w = World::new();
    w.track("PROJ-1", "web", "b1");
    w.track("PROJ-2", "web", "b2");
    w.jira.add_ticket(ticket("PROJ-1", "In Review"));
    w.jira.add_ticket(ticket("PROJ-2", "In Review"));
    sync_jira(&w.ctx(1000, false), &w.jira);

    w.jira.remove_ticket(&key("PROJ-1"));
    w.jira.add_ticket(RemoteTicket {
        title: "renamed".into(),
        ..ticket("PROJ-2", "Done")
    });
    let report = sync_jira(&w.ctx(2000, false), &w.jira);
    assert!(matches!(report.outcome, SourceOutcome::Partial(_)));
    assert_eq!(outcome_kind(&report), Some(ProviderErrorKind::NotFound));
    assert_eq!(w.cached_keys(), ["PROJ-1", "PROJ-2"]);
    assert_eq!(
        jira_cache::get(&w.cache, &key("PROJ-2"))
            .unwrap()
            .unwrap()
            .title,
        "renamed"
    );
}

#[test]
fn unset_review_jql_uses_the_default_query_even_without_a_jira_section() {
    let default_jql = "status = \"In Review\" AND updated >= -30d ORDER BY updated DESC";
    let mut w = World::new();
    w.jira
        .on_search(default_jql, vec![ticket("PROJ-1", "In Review")]);

    for text in ["[jira]\nsite = \"x\"\n", ""] {
        w.config = Config::parse(text).unwrap();
        let report = sync_jira(&w.ctx(1000, true), &w.jira);
        assert_eq!(report.outcome, SourceOutcome::Synced, "config: {text:?}");
        assert_eq!(w.cached_keys(), ["PROJ-1"]);
    }
    assert!(w.jira.log().args_of("search").iter().any(|a| a.contains(default_jql)));

    // The default follows the configured review status name.
    w.config =
        Config::parse("[jira.statuses]\nreview = \"Code Review\"\n").unwrap();
    assert!(w.config.review_jql().starts_with("status = \"Code Review\" AND"));
}

#[test]
fn provider_warnings_such_as_truncated_comments_become_report_notes() {
    let w = World::new();
    w.jira
        .on_search(POOL_JQL, vec![ticket("PROJ-1", "In Review")]);
    w.jira
        .set_warnings(vec!["PROJ-1: acli returned 2 of 40 comments".into()]);
    let report = sync_jira(&w.ctx(1000, false), &w.jira);
    assert_eq!(report.outcome, SourceOutcome::Synced);
    assert!(
        report.notes.iter().any(|n| n.contains("2 of 40")),
        "{:?}",
        report.notes
    );
}

// ------------------------------------------------------------- freshness

#[test]
fn a_fresh_source_is_skipped_unless_forced_and_failures_never_count_as_fresh() {
    let w = World::new();
    w.jira
        .on_search(POOL_JQL, vec![ticket("PROJ-1", "In Review")]);

    assert_eq!(
        sync_jira(&w.ctx(1000, false), &w.jira).outcome,
        SourceOutcome::Synced
    );
    let calls = w.jira.log().calls().len();

    let skipped = sync_jira(&w.ctx(1059, false), &w.jira);
    assert_eq!(skipped.outcome, SourceOutcome::Skipped { last_ok_at: 1000 });
    assert_eq!(
        w.jira.log().calls().len(),
        calls,
        "a skipped sync makes no calls"
    );

    assert_eq!(
        sync_jira(&w.ctx(1059, true), &w.jira).outcome,
        SourceOutcome::Synced
    );
    assert_eq!(
        sync_jira(&w.ctx(1200, false), &w.jira).outcome,
        SourceOutcome::Synced
    );

    // A failed attempt does not make the source fresh.
    w.jira.set_offline();
    assert!(matches!(
        sync_jira(&w.ctx(2000, false), &w.jira).outcome,
        SourceOutcome::Failed(_)
    ));
    w.jira.set_failure(None);
    assert_eq!(
        sync_jira(&w.ctx(2001, false), &w.jira).outcome,
        SourceOutcome::Synced
    );
}

// ------------------------------------------------------------ code host

fn seed_pool(w: &World, keys: &[&str]) {
    for k in keys {
        jira_cache::upsert_remote(&w.cache, &ticket(k, "In Review"), 1).unwrap();
    }
}

#[test]
fn open_prs_of_interest_are_cached_with_details_and_comments() {
    let w = World::new();
    seed_pool(&w, &["PROJ-1"]);
    w.track("PROJ-2", "web", "feature/PROJ-2");
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1-a", "develop"));
    w.bb.add_pr(pr("acme/web", 2, "feature/PROJ-2-b", "master"));
    // Neither pool nor tracked (and PROJ-12 must not match PROJ-1).
    w.bb.add_pr(pr("acme/web", 3, "feature/PROJ-12-c", "develop"));
    w.bb.add_pr(pr("acme/web", 4, "unrelated", "develop"));
    let mut merged = pr("acme/web", 5, "feature/PROJ-1-old", "develop");
    merged.state = PrState::Merged;
    w.bb.add_pr(merged);
    w.bb.set_pr_comments(
        "acme/web",
        1,
        vec![crate::providers::PrComment {
            id: 10,
            pr: 1,
            author: "a".into(),
            body: "looks off".into(),
            inline: None,
            created_at: 5,
        }],
    );

    let reports = sync_code_host(
        &w.ctx(1000, false),
        &w.bb,
        &[hosted("web", "acme/web")],
        &[],
    );
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].outcome,
        SourceOutcome::Synced,
        "{:?}",
        reports[0]
    );

    let cached: Vec<u64> = prs::list_for_repo(&w.cache, "acme/web")
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(cached, [1, 2]);
    assert_eq!(prs::comments(&w.cache, "acme/web", 1).unwrap().len(), 1);
    assert_eq!(w.bb.log().count("pr"), 2);
    assert!(w.bb.log().writes().is_empty());
}

#[test]
fn a_pr_that_disappears_is_reconciled_but_only_on_success() {
    let w = World::new();
    w.track("PROJ-1", "web", "feature/PROJ-1");
    w.track("PROJ-2", "web", "feature/PROJ-2");
    w.track("PROJ-3", "web", "feature/PROJ-3");
    for (id, branch) in [
        (1, "feature/PROJ-1"),
        (2, "feature/PROJ-2"),
        (3, "feature/PROJ-3"),
    ] {
        w.bb.add_pr(pr("acme/web", id, branch, "develop"));
    }
    let repos = [hosted("web", "acme/web")];
    sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &[]);
    assert_eq!(prs::list_all(&w.cache).unwrap().len(), 3);

    // While offline nothing is removed, however the remote changed meanwhile.
    w.bb.remove_pr("acme/web", 1);
    w.bb.set_offline();
    let r = sync_code_host(&w.ctx(2000, false), &w.bb, &repos, &[]);
    assert!(matches!(r[0].outcome, SourceOutcome::Failed(_)));
    assert_eq!(outcome_kind(&r[0]), Some(ProviderErrorKind::Network));
    assert_eq!(prs::list_all(&w.cache).unwrap().len(), 3);
    w.bb.set_failure(None);

    // PR 1 vanished (deleted), PR 2 was merged, PR 3 is still open.
    let mut merged = pr("acme/web", 2, "feature/PROJ-2", "develop");
    merged.state = PrState::Merged;
    w.bb.add_pr(merged);
    let r = sync_code_host(&w.ctx(3000, false), &w.bb, &repos, &[]);
    assert_eq!(r[0].outcome, SourceOutcome::Synced, "{:?}", r[0]);
    assert!(prs::get(&w.cache, "acme/web", 1).unwrap().is_none());
    assert_eq!(
        prs::get(&w.cache, "acme/web", 2).unwrap().unwrap().state,
        PrState::Merged
    );
    assert_eq!(
        prs::get(&w.cache, "acme/web", 3).unwrap().unwrap().state,
        PrState::Open
    );
}

#[test]
fn a_reconcile_error_other_than_not_found_keeps_the_row() {
    let w = World::new();
    w.track("PROJ-1", "web", "feature/PROJ-1");
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    let repos = [hosted("web", "acme/web")];
    sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &[]);

    // The PR is gone from the open list, and looking it up fails oddly.
    w.bb.remove_pr("acme/web", 1);
    w.bb.fail_pr_lookup(Some(ProviderError::Other("weird".into())));
    let r = sync_code_host(&w.ctx(2000, false), &w.bb, &repos, &[]);
    assert!(
        matches!(r[0].outcome, SourceOutcome::Partial(_)),
        "{:?}",
        r[0]
    );
    assert!(prs::get(&w.cache, "acme/web", 1).unwrap().is_some());

    // Once the lookup works it is a clean NotFound and the row goes.
    w.bb.fail_pr_lookup(None);
    let r = sync_code_host(&w.ctx(3000, false), &w.bb, &repos, &[]);
    assert_eq!(r[0].outcome, SourceOutcome::Synced);
    assert!(prs::get(&w.cache, "acme/web", 1).unwrap().is_none());
}

#[test]
fn pipelines_are_synced_for_tracked_branches_and_uat_commits() {
    let w = World::new();
    w.track("PROJ-1", "web", "feature/PROJ-1");
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    w.bb.add_pipeline(run(
        "acme/web",
        "p-branch",
        "feature/PROJ-1",
        "1111111aaaa",
        10,
    ));
    w.bb.add_pipeline(run("acme/web", "p-uat", "uat", "abcdef1234567890", 20));
    // Unrelated branch and unrelated repo: never fetched.
    w.bb.add_pipeline(run(
        "acme/web",
        "p-other",
        "feature/other",
        "2222222bbbb",
        30,
    ));
    w.bb.add_pipeline(run("acme/api", "p-api", "uat", "abcdef1234567890", 40));
    w.bb.strip_steps_in_lists(true);

    // The integration flow recorded a uat merge for the web project.
    tickets::set_status(
        &w.state,
        &key("PROJ-1"),
        crate::domain::LocalStatus::Active,
        2,
    )
    .unwrap();
    uat_merges::record(
        &w.state,
        &uat_merges::UatMerge {
            ticket: key("PROJ-1"),
            repo: "web".into(),
            branch: "uat".into(),
            commit: "abcdef1234567890".into(),
            recorded_at: 3,
        },
    )
    .unwrap();
    let repos = [hosted("web", "acme/web")];
    let commits = uat_commits(&w.state, &repos).unwrap();
    assert_eq!(
        commits,
        [("acme/web".to_string(), "abcdef1234567890".to_string())]
    );

    let r = sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &commits);
    assert_eq!(r[0].outcome, SourceOutcome::Synced, "{:?}", r[0]);

    let ids: Vec<String> = pipelines::list_for_repo(&w.cache, "acme/web")
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ["p-uat", "p-branch"]);
    // Steps came from the per-run detail fetch even though lists stripped them.
    let uat_run = &pipelines::for_commit(&w.cache, "acme/web", "abcdef1").unwrap()[0];
    assert!(uat_run.deployed(Some("alpha")));
    assert!(
        pipelines::for_commit(&w.cache, "acme/api", "abcdef1")
            .unwrap()
            .is_empty()
    );

    // A second sync of finished runs does not refetch their details.
    let details = w.bb.log().count("pipeline");
    sync_code_host(&w.ctx(2000, true), &w.bb, &repos, &commits);
    assert_eq!(w.bb.log().count("pipeline"), details);
}

#[test]
fn a_running_pipeline_is_refreshed_until_it_finishes() {
    let w = World::new();
    w.track("PROJ-1", "web", "feature/PROJ-1");
    let repos = [hosted("web", "acme/web")];
    let mut r = run("acme/web", "p1", "feature/PROJ-1", "1111111aaaa", 10);
    r.state = PipelineState::Running;
    r.completed_at = None;
    r.steps = vec![PipelineStep {
        name: "deploy".into(),
        state: PipelineState::Running,
        deployment_environment: Some("alpha".into()),
    }];
    w.bb.add_pipeline(r.clone());
    sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &[]);
    assert!(
        !pipelines::get(&w.cache, "acme/web", "p1")
            .unwrap()
            .unwrap()
            .deployed(None)
    );

    r.state = PipelineState::Succeeded;
    r.steps[0].state = PipelineState::Succeeded;
    w.bb.add_pipeline(r);
    sync_code_host(&w.ctx(2000, true), &w.bb, &repos, &[]);
    assert!(
        pipelines::get(&w.cache, "acme/web", "p1")
            .unwrap()
            .unwrap()
            .deployed(None)
    );
}

// -------------------------------------------------------- failure isolation

#[test]
fn one_failing_repo_does_not_stop_the_others() {
    let w = World::new();
    for (k, repo) in [("PROJ-1", "web"), ("PROJ-2", "api"), ("PROJ-3", "worker")] {
        w.track(k, repo, &format!("feature/{k}"));
        w.bb.add_pr(pr(
            &format!("acme/{repo}"),
            1,
            &format!("feature/{k}"),
            "develop",
        ));
    }
    let repos = [
        hosted("web", "acme/web"),
        hosted("api", "acme/api"),
        hosted("worker", "acme/worker"),
    ];

    // Seed api's cache, then make it fail: its rows must survive.
    sync_code_host(&w.ctx(500, false), &w.bb, &repos[1..2], &[]);
    w.bb.fail_repo("acme/api", ProviderError::Network("timeout".into()));

    let reports = sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &[]);
    assert_eq!(reports.len(), 3);
    assert_eq!(reports[0].outcome, SourceOutcome::Synced);
    assert_eq!(outcome_kind(&reports[1]), Some(ProviderErrorKind::Network));
    assert_eq!(reports[2].outcome, SourceOutcome::Synced);
    assert_eq!(reports[1].source, "bitbucket:acme/api");

    assert_eq!(prs::list_for_repo(&w.cache, "acme/web").unwrap().len(), 1);
    assert_eq!(prs::list_for_repo(&w.cache, "acme/api").unwrap().len(), 1);
    assert_eq!(
        prs::list_for_repo(&w.cache, "acme/worker").unwrap().len(),
        1
    );
    assert!(
        sync_state::get(&w.cache, "bitbucket:acme/api")
            .unwrap()
            .unwrap()
            .last_error
            .is_some()
    );
    assert!(
        sync_state::get(&w.cache, "bitbucket:acme/web")
            .unwrap()
            .unwrap()
            .last_error
            .is_none()
    );
}

#[test]
fn a_non_environmental_repo_failure_is_partial_and_isolated() {
    let w = World::new();
    w.track("PROJ-1", "web", "feature/PROJ-1");
    w.track("PROJ-2", "api", "feature/PROJ-2");
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    w.bb.add_pr(pr("acme/api", 1, "feature/PROJ-2", "develop"));
    w.bb.fail_repo("acme/web", ProviderError::Other("boom".into()));

    let repos = [hosted("web", "acme/web"), hosted("api", "acme/api")];
    let reports = sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &[]);
    assert!(matches!(reports[0].outcome, SourceOutcome::Partial(_)));
    assert_eq!(reports[1].outcome, SourceOutcome::Synced);
    assert!(prs::list_for_repo(&w.cache, "acme/web").unwrap().is_empty());
    assert_eq!(prs::list_for_repo(&w.cache, "acme/api").unwrap().len(), 1);
}

#[test]
fn sync_all_isolates_sources_and_reports_each() {
    let w = World::new();
    w.jira
        .on_search(POOL_JQL, vec![ticket("PROJ-1", "In Review")]);
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    let repos = [hosted("web", "acme/web")];

    // Jira down, Bitbucket fine: the PR of the (uncached) pool ticket is not of interest yet,
    // but the source itself synced.
    w.jira.set_offline();
    let report = sync_all(
        &w.ctx(1000, false),
        Ok(&w.jira as &dyn TicketProvider),
        Ok(&w.bb as &dyn CodeHost),
        &repos,
        &[],
        None,
    );
    assert_eq!(report.sources.len(), 2);
    assert_eq!(report.sources[0].system, SyncSource::Jira);
    assert_eq!(
        outcome_kind(&report.sources[0]),
        Some(ProviderErrorKind::Network)
    );
    assert_eq!(report.sources[1].outcome, SourceOutcome::Synced);
    assert!(!report.is_ok());
    assert_eq!(report.failures().count(), 1);

    // Jira back: the next sync finds the pool ticket and, on the code side, its PR.
    w.jira.set_failure(None);
    let report = sync_all(
        &w.ctx(2000, false),
        Ok(&w.jira as &dyn TicketProvider),
        Ok(&w.bb as &dyn CodeHost),
        &repos,
        &[],
        None,
    );
    assert_eq!(report.sources[0].outcome, SourceOutcome::Synced);
    // Bitbucket ran only 1000s ago... which is beyond the 60s interval, so it synced again.
    assert_eq!(report.sources[1].outcome, SourceOutcome::Synced);
    assert_eq!(prs::for_ticket(&w.cache, &key("PROJ-1")).unwrap().len(), 1);
}

#[test]
fn sync_all_reports_unavailable_adapters_and_honours_only() {
    let w = World::new();
    let missing = ProviderError::not_installed("acli", "adapter not available in this build");
    let missing_bb = ProviderError::not_installed("bkt", "adapter not available in this build");
    let repos = [hosted("web", "acme/web"), hosted("api", "acme/api")];

    let report = sync_all(
        &w.ctx(1000, false),
        Err(&missing),
        Err(&missing_bb),
        &repos,
        &[],
        None,
    );
    let sources: Vec<&str> = report.sources.iter().map(|s| s.source.as_str()).collect();
    assert_eq!(
        sources,
        ["jira", "bitbucket:acme/web", "bitbucket:acme/api"]
    );
    assert!(
        report
            .sources
            .iter()
            .all(|s| outcome_kind(s) == Some(ProviderErrorKind::NotInstalled))
    );

    // One unavailable adapter does not stop the other source.
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    let report = sync_all(
        &w.ctx(2000, true),
        Err(&missing),
        Ok(&w.bb as &dyn CodeHost),
        &repos,
        &[],
        None,
    );
    assert!(!report.sources[0].is_ok());
    assert_eq!(report.sources[1].outcome, SourceOutcome::Synced);

    // --only jira: the code host is not even looked at.
    let before = w.bb.log().calls().len();
    let report = sync_all(
        &w.ctx(3000, true),
        Ok(&w.jira as &dyn TicketProvider),
        Ok(&w.bb as &dyn CodeHost),
        &repos,
        &[],
        Some(SyncSource::Jira),
    );
    assert_eq!(report.sources.len(), 1);
    assert_eq!(w.bb.log().calls().len(), before);

    // No hosted repos: reported as not configured rather than silently absent.
    let report = sync_all(
        &w.ctx(4000, true),
        Ok(&w.jira as &dyn TicketProvider),
        Ok(&w.bb as &dyn CodeHost),
        &[],
        &[],
        Some(SyncSource::Bitbucket),
    );
    assert!(matches!(
        report.sources[0].outcome,
        SourceOutcome::NotConfigured(_)
    ));

    // Jira needs no config section (acli owns the login), so an unavailable adapter is a failure.
    let mut bare = World::new();
    bare.config = Config::default();
    let report = sync_all(
        &bare.ctx(1, false),
        Err(&missing),
        Err(&missing_bb),
        &[],
        &[],
        None,
    );
    assert!(!report.is_ok());
    assert_eq!(
        report.sources[0].problem().map(|p| p.kind),
        Some(ProviderErrorKind::NotInstalled)
    );
}

#[test]
fn sync_never_calls_a_write_method() {
    let w = World::new();
    w.jira
        .on_search(POOL_JQL, vec![ticket("PROJ-1", "In Review")]);
    w.jira
        .on_search(RETURNED_JQL, vec![ticket("PROJ-2", "Returned")]);
    w.track("PROJ-3", "web", "feature/PROJ-3");
    w.jira.add_ticket(ticket("PROJ-3", "In Review"));
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    w.bb.add_pr(pr("acme/web", 2, "feature/PROJ-3", "master"));
    w.bb.add_pipeline(run("acme/web", "p1", "feature/PROJ-3", "1111111aaaa", 1));
    let repos = [hosted("web", "acme/web")];
    let commits = vec![("acme/web".to_string(), "1111111aaaa".to_string())];

    for now in [1000, 2000] {
        let ctx = w.ctx(now, true);
        sync_all(
            &ctx,
            Ok(&w.jira as &dyn TicketProvider),
            Ok(&w.bb as &dyn CodeHost),
            &repos,
            &commits,
            None,
        );
    }
    // Break things too: failing syncs must not write either.
    w.jira.set_offline();
    w.bb.set_offline();
    sync_all(
        &w.ctx(3000, true),
        Ok(&w.jira as &dyn TicketProvider),
        Ok(&w.bb as &dyn CodeHost),
        &repos,
        &commits,
        None,
    );

    assert!(!w.jira.log().calls().is_empty() && !w.bb.log().calls().is_empty());
    assert_eq!(w.jira.log().writes(), []);
    assert_eq!(w.bb.log().writes(), []);
    for call in w.jira.log().calls().into_iter().chain(w.bb.log().calls()) {
        assert!(!call.is_write(), "{call:?}");
    }
}

#[test]
fn hotfix_kind_follows_synced_prs() {
    let w = World::new();
    w.track("PROJ-1", "web", "feature/PROJ-1");
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "master"));
    let repos = [hosted("web", "acme/web")];
    sync_code_host(&w.ctx(1000, false), &w.bb, &repos, &[]);
    let k: TicketKey = key("PROJ-1");
    let d = ticket_kind(&w.state, &w.cache, &k, &repos).unwrap();
    assert_eq!(d.kind, crate::domain::TicketKind::Hotfix);

    // The PR is retargeted to develop: normal again after the next sync.
    w.bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
    sync_code_host(&w.ctx(2000, true), &w.bb, &repos, &[]);
    let d = ticket_kind(&w.state, &w.cache, &k, &repos).unwrap();
    assert_eq!(d.kind, crate::domain::TicketKind::Normal);
}

//! Tests of the provider mirror in `cache.db` and of the M4 migrations.

use rusqlite::Connection;

use super::*;
use crate::domain::TicketKey;
use crate::providers::fake::build::{comment, key, pr, run};
use crate::providers::{
    DiffSide, InlineAnchor, PipelineState, PipelineStep, Pr, PrComment, PrState, Reviewer,
};

fn cache() -> Store {
    Store::open_in_memory(Kind::Cache).unwrap()
}

fn reviewer(account: &str, approved: bool) -> Reviewer {
    Reviewer {
        account: account.into(),
        approved,
        changes_requested: false,
    }
}

// ----------------------------------------------------------- migrations

#[test]
fn cache_upgrades_from_every_earlier_version_keeping_data() {
    let latest = Store::open_in_memory(Kind::Cache)
        .unwrap()
        .schema_version()
        .unwrap() as usize;
    assert!(latest >= 3);
    for from in 1..latest {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut conn = Connection::open(dir.path().join("cache.db")).unwrap();
            migrations::for_kind(Kind::Cache)
                .to_version(&mut conn, from)
                .unwrap();
            if from >= 2 {
                conn.execute(
                    "INSERT INTO jira_tickets (key, title, jira_status, raw_json, fetched_at)
                     VALUES ('PROJ-1', 'kept', 'In Review', '{}', 1)",
                    [],
                )
                .unwrap();
            }
        }

        let store = Store::open_in(dir.path(), Kind::Cache).unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, latest);
        if from >= 2 {
            let t = jira_cache::get(&store, &key("PROJ-1")).unwrap().unwrap();
            assert_eq!(t.title, "kept");
        }
        // The new tables are usable after the upgrade.
        prs::upsert(&store, &pr("acme/web", 1, "b", "develop"), 1).unwrap();
        sync_state::record_ok(&store, "jira", 1).unwrap();
    }
}

#[test]
fn state_upgrades_to_uat_merges_keeping_tickets_and_activation() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut conn = Connection::open(dir.path().join("state.db")).unwrap();
        migrations::for_kind(Kind::State)
            .to_version(&mut conn, 3)
            .unwrap();
        conn.execute(
            "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at)
             VALUES ('PROJ-1', 'active', 0, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO activation_repos (ticket_key, repo, position, repo_dir, role, branch, created_at)
             VALUES ('PROJ-1', 'web', 0, '/tmp/web', 'ticket', 'feature/PROJ-1', 1)",
            [],
        )
        .unwrap();
    }

    let store = Store::open_in(dir.path(), Kind::State).unwrap();
    assert_eq!(store.schema_version().unwrap(), 7);
    let t = tickets::get(&store, &key("PROJ-1")).unwrap().unwrap();
    assert_eq!(t.status, crate::domain::LocalStatus::Active);
    assert_eq!(restore::list(&store, &key("PROJ-1")).unwrap().len(), 1);

    let merge = uat_merges::UatMerge {
        ticket: key("PROJ-1"),
        repo: "web".into(),
        branch: "uat".into(),
        commit: "abcdef1234567".into(),
        recorded_at: 5,
    };
    uat_merges::record(&store, &merge).unwrap();
    uat_merges::record(&store, &merge).unwrap();
    assert_eq!(
        uat_merges::list(&store).unwrap(),
        std::slice::from_ref(&merge)
    );
    assert_eq!(
        uat_merges::list_for_ticket(&store, &key("PROJ-2")).unwrap(),
        []
    );
}

#[test]
fn pr_state_check_constraint_mirrors_the_enum() {
    let store = cache();
    for (i, state) in PrState::ALL.iter().enumerate() {
        let mut p = pr("acme/web", i as u64 + 1, "b", "develop");
        p.state = *state;
        prs::upsert(&store, &p, 1).unwrap();
        assert_eq!(
            prs::get(&store, "acme/web", i as u64 + 1)
                .unwrap()
                .unwrap()
                .state,
            *state
        );
    }
    let bad = store.conn().execute(
        "INSERT INTO prs VALUES ('r', 1, 't', 'closed', 'a', 'b', 'c', 'u', 1, 1)",
        [],
    );
    assert!(bad.is_err());
}

// ------------------------------------------------------------------ PRs

#[test]
fn pr_upsert_is_idempotent_and_replaces_reviewers() {
    let store = cache();
    let mut p = pr("acme/web", 7, "feature/PROJ-1", "develop");
    p.reviewers = vec![reviewer("a", false), reviewer("b", true)];

    prs::upsert(&store, &p, 10).unwrap();
    prs::upsert(&store, &p, 11).unwrap();
    assert_eq!(prs::list_all(&store).unwrap().len(), 1);
    assert_eq!(prs::get(&store, "acme/web", 7).unwrap().unwrap(), p);

    p.title = "New title".into();
    p.reviewers = vec![reviewer("b", true)];
    prs::upsert(&store, &p, 12).unwrap();
    let got = prs::get(&store, "acme/web", 7).unwrap().unwrap();
    assert_eq!(got.title, "New title");
    assert_eq!(got.reviewers, [reviewer("b", true)]);
    assert_eq!(prs::get(&store, "acme/web", 8).unwrap(), None);
}

#[test]
fn prs_for_ticket_respect_key_boundaries_and_use_branch_or_title() {
    let store = cache();
    prs::upsert(
        &store,
        &pr("acme/web", 1, "feature/PROJ-1-login", "develop"),
        1,
    )
    .unwrap();
    prs::upsert(
        &store,
        &pr("acme/web", 2, "feature/PROJ-12-other", "develop"),
        1,
    )
    .unwrap();
    let mut by_title = pr("acme/api", 3, "some-branch", "develop");
    by_title.title = "proj-1: fix the thing".into();
    prs::upsert(&store, &by_title, 1).unwrap();
    let mut merged = pr("acme/web", 4, "PROJ-1-old", "develop");
    merged.state = PrState::Merged;
    prs::upsert(&store, &merged, 1).unwrap();
    // A longer project prefix is a different key.
    prs::upsert(&store, &pr("acme/web", 5, "XPROJ-1", "develop"), 1).unwrap();

    let ids = |v: Vec<Pr>| v.into_iter().map(|p| (p.repo, p.id)).collect::<Vec<_>>();
    assert_eq!(
        ids(prs::for_ticket(&store, &key("PROJ-1")).unwrap()),
        [
            ("acme/api".into(), 3),
            ("acme/web".into(), 1),
            ("acme/web".into(), 4)
        ]
    );
    assert_eq!(
        ids(prs::open_for_ticket(&store, &key("PROJ-1")).unwrap()),
        [("acme/api".into(), 3), ("acme/web".into(), 1)]
    );
    assert_eq!(
        ids(prs::for_ticket(&store, &key("PROJ-12")).unwrap()),
        [("acme/web".into(), 2)]
    );
    assert!(
        prs::for_ticket(&store, &key("PROJ-123"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pr_comments_are_replaced_as_a_set_and_deleted_with_the_pr() {
    let store = cache();
    prs::upsert(&store, &pr("acme/web", 1, "b", "develop"), 1).unwrap();
    let general = PrComment {
        id: 1,
        pr: 1,
        author: "a".into(),
        body: "hello".into(),
        inline: None,
        created_at: 5,
    };
    let inline = PrComment {
        id: 2,
        pr: 1,
        author: "b".into(),
        body: "here".into(),
        inline: Some(InlineAnchor {
            path: "src/a.rs".into(),
            line: 12,
            side: DiffSide::New,
        }),
        created_at: 6,
    };
    prs::replace_comments(&store, "acme/web", 1, &[general.clone(), inline.clone()]).unwrap();
    prs::replace_comments(&store, "acme/web", 1, &[general.clone(), inline.clone()]).unwrap();
    assert_eq!(
        prs::comments(&store, "acme/web", 1).unwrap(),
        [general.clone(), inline]
    );

    prs::replace_comments(&store, "acme/web", 1, std::slice::from_ref(&general)).unwrap();
    assert_eq!(prs::comments(&store, "acme/web", 1).unwrap(), [general]);

    assert!(prs::delete(&store, "acme/web", 1).unwrap());
    assert!(prs::comments(&store, "acme/web", 1).unwrap().is_empty());
    assert!(!prs::delete(&store, "acme/web", 1).unwrap());
}

// ------------------------------------------------------------ pipelines

#[test]
fn pipeline_upsert_is_idempotent_and_keeps_steps_a_list_fetch_lacks() {
    let store = cache();
    let full = run("acme/web", "p1", "uat", "abcdef1234567890", 10);
    pipelines::upsert(&store, &full, 1).unwrap();
    pipelines::upsert(&store, &full, 2).unwrap();
    assert_eq!(
        pipelines::list_for_repo(&store, "acme/web").unwrap(),
        std::slice::from_ref(&full)
    );

    // A list result (no steps) updates the run but keeps the steps from the detail fetch.
    let mut listed = full.clone();
    listed.steps.clear();
    listed.state = PipelineState::Failed;
    pipelines::upsert(&store, &listed, 3).unwrap();
    let got = pipelines::get(&store, "acme/web", "p1").unwrap().unwrap();
    assert_eq!(got.state, PipelineState::Failed);
    assert_eq!(got.steps, full.steps);

    // A detail fetch replaces them.
    let mut detailed = full.clone();
    detailed.steps = vec![PipelineStep {
        name: "build".into(),
        state: PipelineState::Other("PAUSED".into()),
        deployment_environment: None,
    }];
    pipelines::upsert(&store, &detailed, 4).unwrap();
    assert_eq!(
        pipelines::get(&store, "acme/web", "p1")
            .unwrap()
            .unwrap()
            .steps,
        detailed.steps
    );
}

#[test]
fn pipelines_are_found_by_commit_and_branch() {
    let store = cache();
    pipelines::upsert(
        &store,
        &run("acme/web", "a", "uat", "abcdef1234567890", 1),
        1,
    )
    .unwrap();
    pipelines::upsert(
        &store,
        &run("acme/web", "b", "uat", "1111111222222333", 2),
        1,
    )
    .unwrap();
    pipelines::upsert(
        &store,
        &run("acme/api", "c", "uat", "abcdef1234567890", 3),
        1,
    )
    .unwrap();
    pipelines::upsert(
        &store,
        &run("acme/web", "d", "feature/x", "abcdef1234567890", 4),
        1,
    )
    .unwrap();

    let ids =
        |v: Vec<crate::providers::PipelineRun>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    // Newest first; the other repo's run is not included; abbreviations match.
    assert_eq!(
        ids(pipelines::for_commit(&store, "acme/web", "abcdef1").unwrap()),
        ["d", "a"]
    );
    assert_eq!(
        ids(pipelines::for_commit(&store, "acme/web", "abcdef1234567890").unwrap()),
        ["d", "a"]
    );
    assert!(
        pipelines::for_commit(&store, "acme/web", "abc")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(pipelines::for_branch(&store, "acme/web", "uat").unwrap()),
        ["b", "a"]
    );
}

// --------------------------------------------------------- Jira comments

#[test]
fn jira_comments_are_replaced_and_mentions_queried() {
    let store = cache();
    let p1 = key("PROJ-1");
    let p2 = key("PROJ-2");
    let mut newer = comment("PROJ-2", "c3", &["me", "you"]);
    newer.created_at = 200;
    jira_comments::replace_for_ticket(
        &store,
        &p1,
        &[
            comment("PROJ-1", "c1", &["me"]),
            comment("PROJ-1", "c2", &["you"]),
        ],
        1,
    )
    .unwrap();
    jira_comments::replace_for_ticket(&store, &p2, std::slice::from_ref(&newer), 1).unwrap();
    // Idempotent.
    jira_comments::replace_for_ticket(&store, &p2, std::slice::from_ref(&newer), 2).unwrap();

    let ids =
        |v: Vec<crate::providers::RemoteComment>| v.into_iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(
        ids(jira_comments::mentioning(&store, "me").unwrap()),
        ["c3", "c1"]
    );
    assert_eq!(
        ids(jira_comments::mentioning(&store, "you").unwrap()),
        ["c3", "c2"]
    );
    assert!(
        jira_comments::mentioning(&store, "nobody")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ids(jira_comments::mentioning_in_ticket(&store, &p1, "me").unwrap()),
        ["c1"]
    );
    assert_eq!(
        jira_comments::list_for_ticket(&store, &p2).unwrap()[0].mentions,
        ["me", "you"]
    );

    // Replacing drops comments (and their mentions) that are gone.
    jira_comments::replace_for_ticket(&store, &p1, &[comment("PROJ-1", "c2", &["you"])], 3)
        .unwrap();
    assert_eq!(
        ids(jira_comments::mentioning(&store, "me").unwrap()),
        ["c3"]
    );

    let keep: Vec<TicketKey> = vec![p1.clone()];
    assert_eq!(jira_comments::delete_except(&store, &keep).unwrap(), 1);
    assert!(
        jira_comments::list_for_ticket(&store, &p2)
            .unwrap()
            .is_empty()
    );
}

// ------------------------------------------------------------ sync state

#[test]
fn sync_state_tracks_success_failure_and_freshness() {
    let store = cache();
    assert_eq!(sync_state::get(&store, "jira").unwrap(), None);
    assert!(!sync_state::is_fresh(&store, "jira", 100, 60).unwrap());

    sync_state::record_failure(&store, "jira", 50, "offline").unwrap();
    let s = sync_state::get(&store, "jira").unwrap().unwrap();
    assert_eq!((s.last_ok_at, s.last_attempt_at), (None, 50));
    assert_eq!(s.last_error.as_deref(), Some("offline"));
    assert!(!sync_state::is_fresh(&store, "jira", 51, 60).unwrap());

    sync_state::record_ok(&store, "jira", 100).unwrap();
    assert!(sync_state::is_fresh(&store, "jira", 159, 60).unwrap());
    assert!(!sync_state::is_fresh(&store, "jira", 160, 60).unwrap());

    // A later failure keeps last_ok_at and does not make the source look stale-fresh.
    sync_state::record_failure(&store, "jira", 200, "boom").unwrap();
    let s = sync_state::get(&store, "jira").unwrap().unwrap();
    assert_eq!(s.last_ok_at, Some(100));
    assert_eq!(s.last_error.as_deref(), Some("boom"));

    sync_state::record_ok(&store, &sync_state::code_host_source("acme/web"), 5).unwrap();
    assert_eq!(sync_state::list(&store).unwrap().len(), 2);
}

#[test]
fn stale_ticket_cache_rows_are_removed_only_when_asked() {
    let store = cache();
    let t = crate::providers::fake::build::ticket("PROJ-1", "In Review");
    let u = crate::providers::fake::build::ticket("PROJ-2", "In Review");
    jira_cache::upsert_remote(&store, &t, 1).unwrap();
    jira_cache::upsert_remote(&store, &u, 1).unwrap();
    assert_eq!(
        jira_cache::delete_except(&store, &[key("PROJ-1")]).unwrap(),
        1
    );
    assert_eq!(jira_cache::list(&store).unwrap().len(), 1);
    // The raw payload is the provider's ticket as JSON.
    let raw = jira_cache::get(&store, &key("PROJ-1"))
        .unwrap()
        .unwrap()
        .raw_json;
    assert!(raw.contains("Title of PROJ-1"));
}

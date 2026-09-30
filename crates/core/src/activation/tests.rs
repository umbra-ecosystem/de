//! Activation end to end against a workspace of real git repositories (with bare origins), a
//! real file-backed `state.db` and a fake composer.

use super::*;
use crate::{
    domain::{AuditOutcome, BaselineChoice, LocalStatus, RepoLinkOrigin, TicketKey},
    git::FastForward,
    overlay::OverlayError,
    store::{audit, links, overlays, restore, tickets, time},
    testsupport::{FakeRunner, Fixture, Repo, WEB_COMPOSER_JSON, WEB_COMPOSER_LOCK, key},
};

const TICKET_BRANCH: &str = "feature/PROJ-1-api-change";

fn activate_with(
    fx: &Fixture,
    ticket: &str,
    opts: &ActivateOptions,
) -> eyre::Result<ActivationReport> {
    activate(&fx.store, &fx.runner, &key(ticket), &fx.repos(), opts, 1000)
}

fn activate_ticket(fx: &Fixture, ticket: &str) -> eyre::Result<ActivationReport> {
    activate_with(fx, ticket, &ActivateOptions::default())
}

fn deactivate_ticket(fx: &Fixture, ticket: &str) -> DeactivationReport {
    deactivate(
        &fx.store,
        &fx.runner,
        &key(ticket),
        LocalStatus::Parked,
        2000,
    )
    .unwrap()
}

/// `web` on a work-in-progress branch with a modified file and an untracked one, and `docs`
/// (already on its base branch) with an untracked note.
fn make_dirty(fx: &Fixture) {
    fx.web.git(&["switch", "-c", "wip"]);
    fx.web.write("app.php", "<?php // half-done edit\n");
    fx.web.write("scratch.txt", "not committed\n");
    fx.docs.write("notes.txt", "my notes\n");
}

fn status_of(fx: &Fixture, ticket: &str) -> LocalStatus {
    tickets::get(&fx.store, &key(ticket))
        .unwrap()
        .unwrap()
        .status
}

fn audit_actions(fx: &Fixture, ticket: &str) -> Vec<String> {
    let mut entries = audit::list(&fx.store, Some(&key(ticket)), 500).unwrap();
    entries.reverse();
    entries.into_iter().map(|e| e.action).collect()
}

fn no_records(fx: &Fixture) {
    assert!(
        restore::list_all(&fx.store).unwrap().is_empty(),
        "restore records left"
    );
    assert!(
        overlays::list_all(&fx.store).unwrap().is_empty(),
        "overlay backups left"
    );
}

fn stash_messages(repo: &Repo) -> Vec<String> {
    repo.stashes()
}

// ------------------------------------------------------------- discovery

#[test]
fn discovery_records_auto_links_and_never_clobbers_manual_or_excluded_ones() {
    let fx = Fixture::with_ticket();
    let repos = fx.repos();
    let t = key("PROJ-1");

    let found = discover_links(&fx.store, &t, &repos, 5).unwrap();
    assert_eq!(
        found.matches,
        [RepoMatches {
            repo: "api-client".into(),
            branches: vec![TICKET_BRANCH.into()]
        }]
    );
    let stored = links::list(&fx.store, &t).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].origin, RepoLinkOrigin::Auto);
    assert_eq!(stored[0].branch.as_deref(), Some(TICKET_BRANCH));

    // A manual choice for web, an exclusion of api-client, a branch appearing in worker.
    links::add_manual(&fx.store, &t, "web", Some("hand-picked")).unwrap();
    links::exclude(&fx.store, &t, "api-client").unwrap();
    fx.worker.add_branch("PROJ-1-worker", "worker.php", false);
    fx.web.add_branch("PROJ-1-in-web", "app.php", false);

    discover_links(&fx.store, &t, &repos, 6).unwrap();
    let by_repo: std::collections::BTreeMap<_, _> = links::list(&fx.store, &t)
        .unwrap()
        .into_iter()
        .map(|l| (l.repo.clone(), l))
        .collect();
    assert_eq!(by_repo["web"].origin, RepoLinkOrigin::Manual);
    assert_eq!(by_repo["web"].branch.as_deref(), Some("hand-picked"));
    assert_eq!(by_repo["api-client"].origin, RepoLinkOrigin::Excluded);
    assert_eq!(by_repo["worker"].origin, RepoLinkOrigin::Auto);
    assert_eq!(by_repo["worker"].branch.as_deref(), Some("PROJ-1-worker"));
}

#[test]
fn a_repo_with_several_branches_is_linked_without_picking_one() {
    let fx = Fixture::with_ticket();
    fx.api_client
        .add_branch("PROJ-1-second", "src/Client.php", false);

    discover_links(&fx.store, &key("PROJ-1"), &fx.repos(), 5).unwrap();
    let link = &links::list(&fx.store, &key("PROJ-1")).unwrap()[0];
    assert_eq!(
        (link.repo.as_str(), link.branch.as_deref()),
        ("api-client", None)
    );
}

// ------------------------------------------------------------ end to end

#[test]
fn activation_puts_the_right_repos_on_the_right_branches_and_records_everything() {
    let fx = Fixture::with_ticket();
    // The ticket branch exists only on the remote; a local tracking branch gets created.
    fx.api_client.git(&["branch", "-D", TICKET_BRANCH]);
    // worker's base has new upstream commits; web's base has diverged from its upstream.
    fx.web
        .commit("local.txt", "local only\n", "local commit on develop");
    let web_develop_local = fx.web.git(&["rev-parse", "develop"]);
    make_dirty(&fx);
    fx.web.advance_origin("develop", "upstream.txt");
    let worker_upstream = fx.worker.advance_origin("main", "worker-upstream.txt");
    let before = fx.snapshot();

    let report = activate_with(
        &fx,
        "PROJ-1",
        &ActivateOptions {
            fetch: true,
            ..ActivateOptions::default()
        },
    )
    .unwrap();

    // Branches: the provider on the ticket branch, everything else on its own baseline.
    assert_eq!(fx.api_client.branch(), TICKET_BRANCH);
    assert_eq!(fx.web.branch(), "develop");
    assert_eq!(fx.worker.branch(), "main");
    assert_eq!(fx.docs.branch(), "master");
    // Providers first, then the rest in the order given.
    let order: Vec<&str> = report.repos.iter().map(|r| r.repo.as_str()).collect();
    assert_eq!(order, ["api-client", "web", "worker", "docs"]);

    // Baselines were brought up to date; the diverged one was left alone with a warning.
    assert_eq!(fx.worker.head(), worker_upstream);
    assert!(matches!(
        report.repos[2].fast_forward,
        Some(FastForward::Advanced { .. })
    ));
    assert_eq!(fx.web.git(&["rev-parse", "develop"]), web_develop_local);
    assert!(matches!(
        report.repos[1].fast_forward,
        Some(FastForward::Diverged {
            ahead: 1,
            behind: 1
        })
    ));
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("web:") && w.contains("diverged")),
        "{:?}",
        report.warnings
    );
    assert!(matches!(
        report.repos[3].fast_forward,
        Some(FastForward::UpToDate)
    ));

    // Commands: web's overlay update, then its rebuild, run in web's directory; nothing else.
    assert_eq!(
        fx.runner.log(),
        ["web: composer update acme/api-client", "web: task build-ui"]
    );
    assert!(fx.runner.calls().iter().all(|c| c.dir == fx.web.dir));
    assert_eq!(report.repos[1].overlaid, ["acme/api-client"]);
    assert_eq!(fx.web.read("vendor/state"), "symlinked\n");
    assert_ne!(fx.web.read("composer.lock"), WEB_COMPOSER_LOCK);
    // No other repo is touched by the overlay.
    for other in [&fx.api_client, &fx.worker, &fx.docs] {
        assert!(!other.exists("composer.lock") || other.is_clean());
    }

    // Ticket state: active, timer running, restore points recorded, audit trail written.
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Active);
    let open = time::open_entry(&fx.store).unwrap().expect("timer running");
    assert_eq!((open.ticket.as_str(), open.started_at), ("PROJ-1", 1000));

    let records = restore::list(&fx.store, &key("PROJ-1")).unwrap();
    assert_eq!(
        records.iter().map(|r| r.repo.as_str()).collect::<Vec<_>>(),
        ["api-client", "web", "worker", "docs"]
    );
    let web_record = &records[1];
    assert_eq!(web_record.previous_branch.as_deref(), Some("wip"));
    assert_eq!(
        web_record.previous_commit.as_deref(),
        Some(before[1].2.as_str())
    );
    assert_eq!(web_record.stash.as_ref().unwrap().label, "de:PROJ-1:web");
    assert_eq!(web_record.role, restore::RepoRole::Baseline);
    assert_eq!(records[0].role, restore::RepoRole::Ticket);
    assert_eq!(records[0].branch, TICKET_BRANCH);
    assert!(
        overlays::get(&fx.store, &key("PROJ-1"), "web")
            .unwrap()
            .is_some()
    );

    assert_eq!(
        audit_actions(&fx, "PROJ-1"),
        [
            "links.discovered",
            "activation.start",
            "activation.repo_switched",
            "activation.repo_switched",
            "overlay.apply",
            "activation.repo_switched",
            "activation.repo_switched",
            "activation.complete",
        ]
    );
}

#[test]
fn dirty_trees_are_stashed_under_traceable_labels_and_restored_exactly() {
    let fx = Fixture::with_ticket();
    make_dirty(&fx);
    let before = fx.snapshot();

    activate_ticket(&fx, "PROJ-1").unwrap();

    // Both dirty repos are clean now, with their work in labelled stashes. (web only differs
    // by the two composer files the overlay rewrote.)
    assert!(fx.docs.is_clean());
    assert_eq!(fx.web.git(&["status", "--porcelain"]).lines().count(), 2);
    assert_eq!(fx.web.read("app.php"), "<?php // web\n");
    assert!(!fx.web.exists("scratch.txt"));
    assert!(!fx.docs.exists("notes.txt"));
    assert!(stash_messages(&fx.web)[0].ends_with(": de:PROJ-1:web"));
    assert!(stash_messages(&fx.docs)[0].ends_with(": de:PROJ-1:docs"));
    assert!(fx.worker.stashes().is_empty());

    let report = deactivate_ticket(&fx, "PROJ-1");
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(report.status, Some(LocalStatus::Parked));

    // Everything is exactly where it was, including the untracked files.
    assert_eq!(fx.snapshot(), before);
    assert_eq!(fx.web.read("app.php"), "<?php // half-done edit\n");
    assert_eq!(fx.web.read("scratch.txt"), "not committed\n");
    assert_eq!(fx.docs.read("notes.txt"), "my notes\n");
    for repo in fx.all() {
        assert!(repo.stashes().is_empty(), "{} still has stashes", repo.name);
    }
    assert!(report.repos.iter().all(|r| r.leftover_stash.is_none()));
    assert!(report.repos.iter().filter(|r| r.stash_popped).count() == 2);

    // Overlay gone, composer files untouched from the committed state.
    assert_eq!(
        fx.web
            .git(&["status", "--porcelain", "composer.json", "composer.lock"]),
        ""
    );
    no_records(&fx);
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Parked);
    assert!(time::open_entry(&fx.store).unwrap().is_none());
    let entries = time::list(&fx.store, &key("PROJ-1")).unwrap();
    assert_eq!(
        (entries[0].started_at, entries[0].ended_at),
        (1000, Some(2000))
    );

    let actions = audit_actions(&fx, "PROJ-1");
    assert!(actions.contains(&"overlay.revert".to_string()));
    assert_eq!(actions.last().unwrap(), "deactivation.complete");
    assert_eq!(
        actions
            .iter()
            .filter(|a| *a == "deactivation.repo_restored")
            .count(),
        4
    );
}

#[test]
fn a_detached_head_is_restored_as_a_detached_head() {
    let fx = Fixture::with_ticket();
    let first = fx.docs.head();
    fx.docs.commit("index.md", "# changed\n", "second");
    fx.docs.git(&["switch", "--detach", &first]);
    fx.docs.write("notes.txt", "dirty while detached\n");

    activate_ticket(&fx, "PROJ-1").unwrap();
    assert_eq!(fx.docs.branch(), "master");
    let record = restore::list(&fx.store, &key("PROJ-1"))
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(record.previous_branch, None);
    assert_eq!(record.previous_commit.as_deref(), Some(first.as_str()));

    deactivate_ticket(&fx, "PROJ-1");
    assert_eq!(fx.docs.branch(), "HEAD");
    assert_eq!(fx.docs.head(), first);
    assert_eq!(fx.docs.read("notes.txt"), "dirty while detached\n");
    assert!(fx.docs.stashes().is_empty());
}

#[test]
fn a_repo_whose_base_is_configured_as_main_or_master_goes_there() {
    let fx = Fixture::with_ticket();
    fx.worker.git(&["switch", "-c", "elsewhere"]);
    fx.docs.git(&["switch", "-c", "elsewhere"]);

    activate_ticket(&fx, "PROJ-1").unwrap();
    assert_eq!(fx.worker.branch(), "main");
    assert_eq!(fx.docs.branch(), "master");
    assert_eq!(fx.web.branch(), "develop");
}

#[test]
fn a_ticket_branch_in_the_consumer_and_the_provider_gets_the_overlay_on_the_ticket_branch() {
    let fx = Fixture::with_ticket();
    fx.web
        .add_branch("feature/PROJ-1-web-change", "app.php", false);

    let report = activate_ticket(&fx, "PROJ-1").unwrap();
    assert_eq!(fx.web.branch(), "feature/PROJ-1-web-change");
    assert_eq!(report.repos[1].overlaid, ["acme/api-client"]);
    assert_eq!(
        fx.runner.log(),
        ["web: composer update acme/api-client", "web: task build-ui"]
    );
}

#[test]
fn activate_after_tasks_run_only_where_the_ticket_branch_is() {
    let fx = Fixture::new();
    tickets::claim(&fx.store, &key("PROJ-1"), 1).unwrap();
    fx.worker.add_branch("PROJ-1-worker", "worker.php", false);

    let report = activate_ticket(&fx, "PROJ-1").unwrap();
    // The provider is on its baseline, so web is configured but gets no overlay and no rebuild.
    assert_eq!(fx.runner.log(), ["worker: task warm"]);
    assert_eq!(
        report
            .repos
            .iter()
            .find(|r| r.repo == "worker")
            .unwrap()
            .tasks_run,
        ["task warm"]
    );
    assert!(overlays::list_all(&fx.store).unwrap().is_empty());
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
}

// ------------------------------------------------------------ preconditions

#[test]
fn a_second_activation_is_rejected_before_any_repo_is_touched() {
    let fx = Fixture::with_ticket();
    tickets::claim(&fx.store, &key("PROJ-2"), 1).unwrap();
    fx.web.add_branch("PROJ-2-web", "app.php", false);
    fx.worker.add_branch("PROJ-2-worker", "worker.php", false);

    activate_ticket(&fx, "PROJ-1").unwrap();
    let snapshot = fx.snapshot();
    let calls = fx.runner.calls().len();
    let audit_before = audit::list(&fx.store, None, 500).unwrap().len();

    let err = activate_ticket(&fx, "PROJ-2").unwrap_err();
    assert!(
        format!("{err:#}").contains("PROJ-1 is already active"),
        "{err:#}"
    );

    assert_eq!(fx.snapshot(), snapshot, "no repo may move");
    assert_eq!(fx.runner.calls().len(), calls);
    assert!(restore::list(&fx.store, &key("PROJ-2")).unwrap().is_empty());
    assert_eq!(status_of(&fx, "PROJ-2"), LocalStatus::Claimed);
    // Not even discovery ran or was audited.
    assert_eq!(
        audit::list(&fx.store, None, 500).unwrap().len(),
        audit_before
    );
    assert!(links::list(&fx.store, &key("PROJ-2")).unwrap().is_empty());
}

#[test]
fn untracked_finished_and_already_active_tickets_are_rejected_untouched() {
    let fx = Fixture::with_ticket();
    let snapshot = fx.snapshot();

    let err = activate_ticket(&fx, "PROJ-9").unwrap_err();
    assert!(format!("{err:#}").contains("not tracked"), "{err:#}");

    tickets::set_status(&fx.store, &key("PROJ-1"), LocalStatus::Done, 5).unwrap();
    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    assert!(
        format!("{err:#}").contains("cannot be activated"),
        "{err:#}"
    );
    assert_eq!(fx.snapshot(), snapshot);
    assert!(fx.runner.calls().is_empty());

    tickets::set_status(&fx.store, &key("PROJ-1"), LocalStatus::Claimed, 6).unwrap();
    activate_ticket(&fx, "PROJ-1").unwrap();
    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    assert!(format!("{err:#}").contains("already active"), "{err:#}");
}

#[test]
fn a_stale_overlay_or_restore_point_blocks_activation() {
    let fx = Fixture::with_ticket();
    tickets::claim(&fx.store, &key("PROJ-2"), 1).unwrap();

    // An overlay left behind by some earlier run.
    crate::overlay::apply(
        &fx.store,
        &fx.runner,
        &crate::overlay::ApplyRequest {
            ticket: &key("PROJ-2"),
            repo: "web",
            consumer_dir: &fx.web.dir,
            packages: &[crate::overlay::OverlayPackage {
                package: "acme/api-client".into(),
                provider: "api-client".into(),
                provider_dir: fx.api_client.dir.clone(),
            }],
            rebuild: &[],
        },
        1,
    )
    .unwrap();
    let snapshot = fx.snapshot();
    let calls = fx.runner.calls().len();

    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    assert!(
        format!("{err:#}").contains("PROJ-2 overlay in web"),
        "{err:#}"
    );
    assert_eq!(fx.snapshot(), snapshot);
    assert_eq!(fx.runner.calls().len(), calls);
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Claimed);
}

#[test]
fn ambiguous_branches_fail_before_touching_anything() {
    let fx = Fixture::with_ticket();
    fx.web.add_branch("feature/PROJ-1-old", "app.php", false);
    fx.web.add_branch("feature/PROJ-1-new", "app.php", false);
    make_dirty(&fx);
    let snapshot = fx.snapshot();

    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("web has several branches"), "{text}");
    assert!(
        text.contains("feature/PROJ-1-old") && text.contains("feature/PROJ-1-new"),
        "{text}"
    );

    assert_eq!(fx.snapshot(), snapshot);
    assert!(fx.web.stashes().is_empty());
    assert_eq!(fx.web.read("scratch.txt"), "not committed\n");
    no_records(&fx);
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Claimed);
    assert!(fx.runner.calls().is_empty());
    assert!(!audit_actions(&fx, "PROJ-1").contains(&"activation.start".to_string()));
}

#[test]
fn a_manual_link_resolves_the_ambiguity_and_an_exclusion_hides_a_repo() {
    let fx = Fixture::with_ticket();
    fx.web.add_branch("feature/PROJ-1-old", "app.php", false);
    fx.web.add_branch("feature/PROJ-1-new", "app.php", false);
    links::add_manual(&fx.store, &key("PROJ-1"), "web", Some("feature/PROJ-1-new")).unwrap();
    links::exclude(&fx.store, &key("PROJ-1"), "api-client").unwrap();

    activate_ticket(&fx, "PROJ-1").unwrap();
    assert_eq!(fx.web.branch(), "feature/PROJ-1-new");
    // api-client has the branch but is excluded: baseline, and so no overlay.
    assert_eq!(fx.api_client.branch(), "develop");
    assert!(overlays::list_all(&fx.store).unwrap().is_empty());
    assert_eq!(fx.runner.log(), Vec::<String>::new());
}

#[test]
fn hotfix_baseline_is_the_callers_choice_and_a_missing_one_fails_early() {
    let fx = Fixture::with_ticket();
    let production = ActivateOptions {
        baseline: BaselineChoice::Production,
        ..ActivateOptions::default()
    };
    let snapshot = fx.snapshot();

    // web has neither master nor main.
    let err = activate_with(&fx, "PROJ-1", &production).unwrap_err();
    assert!(
        format!("{err:#}").contains("web has no production branch"),
        "{err:#}"
    );
    assert_eq!(fx.snapshot(), snapshot);

    fx.web.git(&["branch", "master", "develop"]);
    fx.web.git(&["push", "origin", "master"]);
    activate_with(&fx, "PROJ-1", &production).unwrap();
    assert_eq!(fx.web.branch(), "master");
    // worker has no production setting: `master` is absent there, so it tries `main`.
    assert_eq!(fx.worker.branch(), "main");
    assert_eq!(fx.docs.branch(), "master");
    assert_eq!(fx.api_client.branch(), TICKET_BRANCH);
}

#[test]
fn a_task_that_does_not_exist_is_caught_before_anything_moves() {
    let fx = Fixture::with_ticket();
    let manifest = fx
        .web
        .read("de.toml")
        .replace("[\"build-ui\"]", "[\"no-such-task\"]");
    fx.web.write("de.toml", &manifest);
    fx.web.commit_all("typo in the rebuild task");
    let snapshot = fx.snapshot();

    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    assert!(
        format!("{err:#}").contains("web: task 'no-such-task' is not defined"),
        "{err:#}"
    );
    assert_eq!(fx.snapshot(), snapshot);
    no_records(&fx);
}

#[test]
fn a_failed_fetch_and_an_unreadable_repo_are_warnings_not_failures() {
    let fx = Fixture::with_ticket();
    fx.docs
        .git(&["remote", "set-url", "origin", "/definitely/not/here"]);
    let plain = tempfile::tempdir().unwrap();
    std::fs::write(
        plain.path().join("de.toml"),
        "[project]\nname = \"plain\"\n",
    )
    .unwrap();
    let mut repos = fx.repos();
    repos.push(WorkspaceRepo::load("plain", plain.path()).unwrap());

    let report = activate(
        &fx.store,
        &fx.runner,
        &key("PROJ-1"),
        &repos,
        &ActivateOptions {
            fetch: true,
            ..ActivateOptions::default()
        },
        1,
    )
    .unwrap();

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Could not fetch docs")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("plain could not be read")),
        "{:?}",
        report.warnings
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Could not fetch plain")),
        "{:?}",
        report.warnings
    );
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Active);
    assert!(report.repos.iter().all(|r| r.repo != "plain"));
}

// ---------------------------------------------------------------- rollback

/// Every repo is exactly as it was and nothing about the ticket says it is active.
fn assert_rolled_back(fx: &Fixture, before: &[(String, String, String)], ticket: &str) {
    assert_eq!(&fx.snapshot(), before, "branches and heads");
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
    assert_eq!(fx.web.read("composer.lock"), WEB_COMPOSER_LOCK);
    assert_eq!(fx.web.read("app.php"), "<?php // half-done edit\n");
    assert_eq!(fx.web.read("scratch.txt"), "not committed\n");
    assert_eq!(fx.docs.read("notes.txt"), "my notes\n");
    for repo in fx.all() {
        assert!(
            repo.stashes().is_empty(),
            "{} kept a stash: {:?}",
            repo.name,
            repo.stashes()
        );
    }
    assert_ne!(status_of(fx, ticket), LocalStatus::Active);
    assert!(tickets::active(&fx.store).unwrap().is_none());
    assert!(time::open_entry(&fx.store).unwrap().is_none());
    no_records(fx);
}

#[test]
fn a_failing_rebuild_on_the_second_repo_rolls_everything_back() {
    let fx = Fixture::with_ticket();
    make_dirty(&fx);
    // The dirty web edit must not be mistaken for its committed composer files.
    fx.web.write("app.php", "<?php // half-done edit\n");
    let before = fx.snapshot();
    fx.runner.fail_on("task build-ui");

    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    let text = format!("{err:#}");
    assert!(
        text.contains("rolled back api-client, web")
            || text.contains("rolled back web, api-client"),
        "{text}"
    );
    assert!(text.contains("build-ui"), "{text}");

    assert_rolled_back(&fx, &before, "PROJ-1");
    // The overlay was reverted for real: composer install ran after the failed update.
    assert!(
        fx.runner
            .log()
            .contains(&"web: composer install".to_string())
    );
    let actions = audit_actions(&fx, "PROJ-1");
    assert!(actions.contains(&"activation.failed".to_string()));
    let failed = audit::list(&fx.store, Some(&key("PROJ-1")), 1)
        .unwrap()
        .remove(0);
    assert_eq!(failed.action, "activation.failed");
    assert_eq!(failed.outcome, AuditOutcome::Failure);
}

#[test]
fn a_failure_on_the_third_repo_rolls_back_the_earlier_ones_including_the_overlay() {
    let fx = Fixture::with_ticket();
    fx.worker.add_branch("PROJ-1-worker", "worker.php", false);
    make_dirty(&fx);
    let before = fx.snapshot();
    // web overlays fine, then worker's `[activate] after` task fails.
    fx.runner.fail_on("task warm");

    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    assert!(format!("{err:#}").contains("warm"), "{err:#}");

    assert_rolled_back(&fx, &before, "PROJ-1");
    assert_eq!(fx.worker.branch(), "main");
    assert_eq!(fx.api_client.branch(), "develop");
    // The overlay had been applied before the failure, and was reverted.
    let log = fx.runner.log();
    assert!(log.contains(&"web: composer update acme/api-client".to_string()));
    assert!(log.contains(&"web: composer install".to_string()));
}

#[test]
fn a_repo_whose_switch_fails_rolls_back_the_ones_before_it() {
    let fx = Fixture::with_ticket();
    // worker's ticket branch exists on two remotes, which `switch` refuses to choose between.
    fx.worker.add_branch("PROJ-1-worker", "worker.php", true);
    fx.worker.git(&[
        "remote",
        "add",
        "backup",
        fx.worker.origin.to_str().unwrap(),
    ]);
    fx.worker.git(&["fetch", "backup"]);
    make_dirty(&fx);
    let before = fx.snapshot();

    let err = activate_ticket(&fx, "PROJ-1").unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("several remotes"), "{text}");
    assert!(text.contains("rolled back"), "{text}");

    assert_rolled_back(&fx, &before, "PROJ-1");
}

// ------------------------------------------------------------ deactivation

#[test]
fn leftover_changes_from_testing_are_stashed_never_lost() {
    let fx = Fixture::with_ticket();
    activate_ticket(&fx, "PROJ-1").unwrap();

    // Testing leaves things behind on the ticket branch.
    fx.api_client
        .write("src/Client.php", "<?php // tweaked while testing\n");
    fx.api_client.write("testing.log", "output\n");

    let report = deactivate_ticket(&fx, "PROJ-1");
    assert!(report.is_complete());
    let restored = report
        .repos
        .iter()
        .find(|r| r.repo == "api-client")
        .unwrap();
    let leftover = restored.leftover_stash.as_ref().expect("leftover reported");
    assert_eq!(leftover.label, "de:PROJ-1:api-client:leftover");

    assert_eq!(fx.api_client.branch(), "develop");
    assert!(fx.api_client.is_clean());
    assert!(fx.api_client.stashes()[0].ends_with(": de:PROJ-1:api-client:leftover"));

    // ... and it really contains the work, tracked edit and untracked file alike.
    assert_eq!(
        fx.api_client.git(&["show", "stash@{0}:src/Client.php"]),
        "<?php // tweaked while testing"
    );
    assert_eq!(
        fx.api_client.git(&["show", "stash@{0}^3:testing.log"]),
        "output"
    );
}

#[test]
fn a_stash_that_no_longer_applies_is_kept_and_reported() {
    let fx = Fixture::with_ticket();
    make_dirty(&fx);
    activate_ticket(&fx, "PROJ-1").unwrap();

    // While the ticket is active, `wip` gains a commit that conflicts with the stashed edit.
    fx.web.git(&["switch", "wip"]);
    fx.web.commit(
        "app.php",
        "<?php // a different edit\n",
        "conflicting change",
    );
    fx.web.git(&["switch", "develop"]);

    let report = deactivate_ticket(&fx, "PROJ-1");
    let web = report.repos.iter().find(|r| r.repo == "web").unwrap();
    let (stash, why) = web.stash_kept.as_ref().expect("the conflict is reported");
    assert_eq!(stash.label, "de:PROJ-1:web");
    assert!(why.contains("kept"), "{why}");
    assert!(!web.stash_popped);

    // The stash is still there for the user, the branch was restored, the rest finished.
    assert_eq!(fx.web.branch(), "wip");
    assert!(
        stash_messages(&fx.web)
            .iter()
            .any(|m| m.ends_with(": de:PROJ-1:web"))
    );
    assert!(report.is_complete());
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Parked);
    let audited = audit::list(&fx.store, Some(&key("PROJ-1")), 50).unwrap();
    assert!(audited.iter().any(|e| e.action == "deactivation.repo_restored" && e.outcome == AuditOutcome::Failure));
}

#[test]
fn a_failed_deactivation_can_be_retried_and_only_redoes_what_is_left() {
    let fx = Fixture::with_ticket();
    make_dirty(&fx);
    let wip_head = fx.web.git(&["rev-parse", "wip"]);
    let before = fx.snapshot();
    activate_ticket(&fx, "PROJ-1").unwrap();

    // Simulated trouble: web's previous branch has disappeared.
    fx.web.git(&["branch", "-D", "wip"]);

    let first = deactivate_ticket(&fx, "PROJ-1");
    assert!(!first.is_complete());
    assert_eq!(first.failures.len(), 1);
    assert_eq!(first.failures[0].repo, "web");
    assert!(
        first.failures[0].error.contains("wip"),
        "{:?}",
        first.failures
    );
    assert_eq!(first.status, None);

    // api-client is back and forgotten; web keeps its record; the ticket is still active.
    assert_eq!(fx.api_client.branch(), "develop");
    let remaining = restore::list(&fx.store, &key("PROJ-1")).unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|r| r.repo.as_str())
            .collect::<Vec<_>>(),
        ["web"]
    );
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Active);
    assert!(time::open_entry(&fx.store).unwrap().is_some());
    // The overlay was reverted before the trouble started, so composer is clean.
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
    assert!(
        stash_messages(&fx.web)[0].ends_with(": de:PROJ-1:web"),
        "the stash is safe"
    );
    assert!(audit_actions(&fx, "PROJ-1").contains(&"deactivation.incomplete".to_string()));

    // Fix the problem and retry.
    fx.web.git(&["branch", "wip", &wip_head]);
    fx.runner.clear();
    let second = deactivate_ticket(&fx, "PROJ-1");
    assert!(second.is_complete(), "{second:?}");
    assert_eq!(
        second
            .repos
            .iter()
            .map(|r| r.repo.as_str())
            .collect::<Vec<_>>(),
        ["web"]
    );
    assert!(
        second.overlays.is_empty(),
        "the overlay is not reverted twice"
    );
    assert!(fx.runner.calls().is_empty());

    assert_eq!(fx.snapshot(), before);
    assert_eq!(fx.web.read("scratch.txt"), "not committed\n");
    no_records(&fx);
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Parked);
    assert!(time::open_entry(&fx.store).unwrap().is_none());
}

#[test]
fn a_failed_overlay_revert_keeps_that_repo_on_the_ticket_side_for_a_retry() {
    let fx = Fixture::with_ticket();
    let before = fx.snapshot();
    activate_ticket(&fx, "PROJ-1").unwrap();

    fx.runner.fail_on("composer install");
    let first = deactivate_ticket(&fx, "PROJ-1");
    assert!(!first.is_complete());
    assert_eq!(first.failures[0].repo, "web");
    // web's record and backup are both still there; the other repos were restored.
    assert!(
        restore::list(&fx.store, &key("PROJ-1"))
            .unwrap()
            .iter()
            .any(|r| r.repo == "web")
    );
    assert!(
        overlays::get(&fx.store, &key("PROJ-1"), "web")
            .unwrap()
            .is_some()
    );
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Active);
    assert_eq!(
        fx.web.read("composer.json"),
        WEB_COMPOSER_JSON,
        "files were already restored"
    );

    fx.runner.stop_failing();
    let second = deactivate_ticket(&fx, "PROJ-1");
    assert!(second.is_complete(), "{second:?}");
    assert_eq!(fx.snapshot(), before);
    no_records(&fx);
}

#[test]
fn crash_recovery_uses_only_what_was_persisted() {
    let mut fx = Fixture::with_ticket();
    make_dirty(&fx);
    let before = fx.snapshot();
    let (json, lock) = (fx.web.hash("composer.json"), fx.web.hash("composer.lock"));
    activate_ticket(&fx, "PROJ-1").unwrap();
    assert_ne!(fx.web.hash("composer.lock"), lock);

    // Everything in memory is gone: reopen the database files, use a brand-new runner, and
    // do not even hand over the workspace.
    fx.reopen_store();
    let fresh = FakeRunner::new();
    let report = deactivate(&fx.store, &fresh, &key("PROJ-1"), LocalStatus::Parked, 3000).unwrap();
    assert!(report.is_complete(), "{report:?}");

    assert_eq!(fx.snapshot(), before);
    assert_eq!(fx.web.hash("composer.json"), json);
    assert_eq!(fx.web.hash("composer.lock"), lock);
    assert_eq!(fx.web.read("scratch.txt"), "not committed\n");
    assert_eq!(fresh.log(), ["web: composer install"]);
    no_records(&fx);
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Parked);
    let entry = &time::list(&fx.store, &key("PROJ-1")).unwrap()[0];
    assert_eq!(entry.ended_at, Some(3000));
}

#[test]
fn deactivation_validates_the_target_and_the_state_before_touching_anything() {
    let fx = Fixture::with_ticket();

    // Not active and nothing recorded.
    let err = deactivate(
        &fx.store,
        &fx.runner,
        &key("PROJ-1"),
        LocalStatus::Parked,
        1,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("not active"), "{err:#}");
    let err = deactivate(
        &fx.store,
        &fx.runner,
        &key("PROJ-9"),
        LocalStatus::Parked,
        1,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("not tracked"), "{err:#}");

    activate_ticket(&fx, "PROJ-1").unwrap();
    let snapshot = fx.snapshot();
    // Active cannot go back to reviewing or claimed.
    for bad in [
        LocalStatus::Reviewing,
        LocalStatus::Claimed,
        LocalStatus::Active,
    ] {
        let err = deactivate(&fx.store, &fx.runner, &key("PROJ-1"), bad, 1).unwrap_err();
        assert!(
            format!("{err:#}").contains("cannot go from active"),
            "{err:#}"
        );
    }
    assert_eq!(fx.snapshot(), snapshot, "a rejected target moves nothing");
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Active);

    let report = deactivate(
        &fx.store,
        &fx.runner,
        &key("PROJ-1"),
        LocalStatus::Integrated,
        9,
    )
    .unwrap();
    assert_eq!(report.status, Some(LocalStatus::Integrated));
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Integrated);
}

#[test]
fn park_then_activate_again_starts_a_second_timer_entry() {
    let fx = Fixture::with_ticket();
    activate_ticket(&fx, "PROJ-1").unwrap();
    let parked = park(&fx.store, &fx.runner, &key("PROJ-1"), 2000).unwrap();
    assert_eq!(parked.status, Some(LocalStatus::Parked));

    let report = activate(
        &fx.store,
        &fx.runner,
        &key("PROJ-1"),
        &fx.repos(),
        &ActivateOptions::default(),
        3000,
    )
    .unwrap();
    assert_eq!(report.repos.len(), 4);
    assert_eq!(fx.api_client.branch(), TICKET_BRANCH);
    park(&fx.store, &fx.runner, &key("PROJ-1"), 4000).unwrap();

    let entries = time::list(&fx.store, &key("PROJ-1")).unwrap();
    let spans: Vec<(i64, Option<i64>)> =
        entries.iter().map(|e| (e.started_at, e.ended_at)).collect();
    assert_eq!(spans, [(1000, Some(2000)), (3000, Some(4000))]);
    assert_eq!(status_of(&fx, "PROJ-1"), LocalStatus::Parked);
}

#[test]
fn stash_labels_are_traceable() {
    let t: TicketKey = key("PROJ-7");
    assert_eq!(stash_label(&t, "web"), "de:PROJ-7:web");
}

#[test]
fn double_overlay_apply_error_is_typed() {
    // Kept next to the activation tests: the guard that protects a stale overlay surfaces here
    // as the typed error callers can match on.
    let fx = Fixture::with_ticket();
    activate_ticket(&fx, "PROJ-1").unwrap();
    let err = crate::overlay::apply(
        &fx.store,
        &fx.runner,
        &crate::overlay::ApplyRequest {
            ticket: &key("PROJ-1"),
            repo: "web",
            consumer_dir: &fx.web.dir,
            packages: &[],
            rebuild: &[],
        },
        1,
    )
    .unwrap_err();
    assert!(matches!(
        err.downcast_ref::<OverlayError>(),
        Some(OverlayError::AlreadyApplied { .. })
    ));
}

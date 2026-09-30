//! Behaviour of the overlay engine and its guards against real repositories and a real
//! `state.db`, with composer faked (and once stubbed as a real executable).

use std::os::unix::fs::PermissionsExt;

use serde_json::Value;

use super::*;
use crate::{
    domain::TicketKey,
    store::overlays,
    testsupport::{FakeRunner, Fixture, Repo, WEB_COMPOSER_JSON, WEB_COMPOSER_LOCK, key},
};

fn packages(fx: &Fixture) -> Vec<OverlayPackage> {
    vec![OverlayPackage {
        package: "acme/api-client".into(),
        provider: "api-client".into(),
        provider_dir: fx.api_client.dir.clone(),
    }]
}

fn rebuild(fx: &Fixture) -> Vec<ExternalCommand> {
    vec![ExternalCommand::new(
        &fx.web.dir,
        "sh",
        &["-c", "true"],
        "task build-ui",
    )]
}

fn apply_web(
    fx: &Fixture,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
) -> eyre::Result<ApplyOutcome> {
    apply(
        &fx.store,
        runner,
        &ApplyRequest {
            ticket,
            repo: "web",
            consumer_dir: &fx.web.dir,
            packages: &packages(fx),
            rebuild: &rebuild(fx),
        },
        500,
    )
}

fn fixture() -> Fixture {
    let fx = Fixture::new();
    for k in ["PROJ-1", "PROJ-2"] {
        crate::store::tickets::claim(&fx.store, &key(k), 100).unwrap();
    }
    fx
}

fn json_of(repo: &Repo, rel: &str) -> Value {
    serde_json::from_str(&repo.read(rel)).expect("valid JSON")
}

// ------------------------------------------------------------------ apply

#[test]
fn apply_edits_composer_json_and_runs_update_then_rebuild() {
    let fx = fixture();
    let outcome = apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();

    assert_eq!(outcome.urls, ["../api-client"]);
    assert_eq!(outcome.rebuilt, ["task build-ui"]);
    assert_eq!(
        fx.runner.log(),
        ["web: composer update acme/api-client", "web: task build-ui"]
    );

    let doc = json_of(&fx.web, "composer.json");
    assert_eq!(doc["require"]["acme/api-client"], "*");
    assert_eq!(doc["require"]["monolog/monolog"], "^3.0");
    assert_eq!(doc["description"], "Café shop ");

    let repos = doc["repositories"].as_array().unwrap();
    assert_eq!(repos.len(), 2, "the existing repository is kept");
    assert_eq!(repos[0]["type"], "path");
    assert_eq!(repos[0]["url"], "../api-client");
    assert_eq!(repos[0]["options"]["symlink"], true);
    assert_eq!(repos[1]["type"], "composer");

    let keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["name", "description", "require", "repositories", "config"]
    );

    // composer update rewrote the lock, and vendor/ now points at the symlink.
    assert_ne!(fx.web.read("composer.lock"), WEB_COMPOSER_LOCK);
    assert_eq!(fx.web.read("vendor/state"), "symlinked\n");

    // Both sides are recorded: the originals byte for byte, and what the overlay left.
    let backup = overlays::get(&fx.store, &key("PROJ-1"), "web")
        .unwrap()
        .unwrap();
    assert_eq!(backup.composer_json, WEB_COMPOSER_JSON.as_bytes());
    assert_eq!(
        backup.composer_lock.as_deref(),
        Some(WEB_COMPOSER_LOCK.as_bytes())
    );
    assert_eq!(
        backup.json_after.as_deref(),
        Some(fx.web.read("composer.json").as_bytes())
    );
    assert_eq!(
        backup.lock_after.as_deref(),
        Some(fx.web.read("composer.lock").as_bytes())
    );
    assert_eq!(backup.repo_dir, fx.web.dir);
    assert!(!backup.files_restored);
}

#[test]
fn provider_url_is_relative_for_siblings_and_absolute_otherwise() {
    let fx = Fixture::new();
    assert_eq!(
        provider_url(&fx.web.dir, &fx.api_client.dir),
        "../api-client"
    );

    let elsewhere = tempfile::tempdir().unwrap();
    let url = provider_url(&fx.web.dir, elsewhere.path());
    assert!(std::path::Path::new(&url).is_absolute(), "{url}");
    assert_eq!(
        std::fs::canonicalize(&url).unwrap(),
        std::fs::canonicalize(elsewhere.path()).unwrap()
    );
}

#[test]
fn a_problem_with_composer_json_changes_nothing_and_records_nothing() {
    let fx = fixture();

    // The package is not required at all.
    fx.web
        .write("composer.json", "{\"require\": {\"php\": \"^8\"}}\n");
    let before = fx.web.hash("composer.json");
    let err = apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap_err();
    assert!(matches!(
        err.downcast_ref::<OverlayError>(),
        Some(OverlayError::PackageNotRequired(_))
    ));

    // No composer.json.
    std::fs::remove_file(fx.web.dir.join("composer.json")).unwrap();
    let err = apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap_err();
    assert!(matches!(
        err.downcast_ref::<OverlayError>(),
        Some(OverlayError::MissingComposerJson(_))
    ));

    assert!(overlays::list_all(&fx.store).unwrap().is_empty());
    assert!(fx.runner.calls().is_empty());
    assert!(before.is_some());
}

#[test]
fn applying_twice_is_refused_and_the_first_backup_survives() {
    let fx = fixture();
    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();
    let overlaid_json = fx.web.read("composer.json");
    let calls = fx.runner.calls().len();

    // Same ticket again, and another ticket on the same checkout.
    for ticket in ["PROJ-1", "PROJ-2"] {
        let err = apply_web(&fx, &fx.runner, &key(ticket)).unwrap_err();
        match err.downcast_ref::<OverlayError>() {
            Some(OverlayError::AlreadyApplied { by, repo }) => {
                assert_eq!(by.as_str(), "PROJ-1");
                assert_eq!(repo, "web");
            }
            other => panic!("expected AlreadyApplied, got {other:?}"),
        }
    }

    // Nothing ran, nothing changed, and the backup still holds the true originals.
    assert_eq!(fx.runner.calls().len(), calls);
    assert_eq!(fx.web.read("composer.json"), overlaid_json);
    let backup = overlays::get(&fx.store, &key("PROJ-1"), "web")
        .unwrap()
        .unwrap();
    assert_eq!(backup.composer_json, WEB_COMPOSER_JSON.as_bytes());
    assert_eq!(overlays::list_all(&fx.store).unwrap().len(), 1);

    // ... so reverting still gets the real originals back.
    revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
}

// ----------------------------------------------------------------- revert

#[test]
fn revert_restores_both_files_byte_for_byte_and_reinstalls() {
    let fx = fixture();
    let (json_hash, lock_hash) = (fx.web.hash("composer.json"), fx.web.hash("composer.lock"));

    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();
    assert_ne!(fx.web.hash("composer.json"), json_hash);
    assert_ne!(fx.web.hash("composer.lock"), lock_hash);
    fx.runner.clear();

    let outcome = revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(
        outcome,
        RevertOutcome::Reverted {
            lock_removed: false
        }
    );

    assert_eq!(fx.web.hash("composer.json"), json_hash);
    assert_eq!(fx.web.hash("composer.lock"), lock_hash);
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
    assert_eq!(fx.web.read("composer.lock"), WEB_COMPOSER_LOCK);
    // vendor/ was rebuilt from the restored lock, not left on the symlink.
    assert_eq!(fx.runner.log(), ["web: composer install"]);
    assert_eq!(fx.web.read("vendor/state"), "from lock\n");
    assert!(overlays::list_all(&fx.store).unwrap().is_empty());
    assert!(fx.web.is_clean(), "the tree is exactly as committed");
}

#[test]
fn a_lock_created_by_the_overlay_is_deleted_on_revert() {
    let fx = fixture();
    fx.web.git(&["rm", "-q", "composer.lock"]);
    fx.web.git(&["commit", "-q", "-m", "drop the lock"]);
    assert!(!fx.web.exists("composer.lock"));

    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();
    assert!(fx.web.exists("composer.lock"), "composer update wrote one");
    let backup = overlays::get(&fx.store, &key("PROJ-1"), "web")
        .unwrap()
        .unwrap();
    assert_eq!(backup.composer_lock, None, "recorded as did-not-exist");

    // The fake `composer install` creates a lock when none exists, like the real one.
    let outcome = revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(outcome, RevertOutcome::Reverted { lock_removed: true });
    assert!(!fx.web.exists("composer.lock"));
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
    assert!(fx.web.is_clean());
}

#[test]
fn revert_is_idempotent_and_a_missing_backup_is_a_clear_no_op() {
    let fx = fixture();

    // Nothing applied: nothing happens.
    let outcome = revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(outcome, RevertOutcome::NothingToRevert);
    assert!(fx.runner.calls().is_empty());

    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();
    assert!(matches!(
        revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap(),
        RevertOutcome::Reverted { .. }
    ));
    fx.runner.clear();

    // The second run does nothing, in particular no second composer install.
    assert_eq!(
        revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap(),
        RevertOutcome::NothingToRevert
    );
    assert!(fx.runner.calls().is_empty());
    assert_eq!(fx.web.read("composer.json"), WEB_COMPOSER_JSON);
}

#[test]
fn revert_works_from_a_fresh_process_using_only_what_is_persisted() {
    let mut fx = fixture();
    let (json_hash, lock_hash) = (fx.web.hash("composer.json"), fx.web.hash("composer.lock"));
    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();

    // The "crash": the open database and the runner are gone; only the files on disk remain.
    fx.reopen_store();
    let fresh_runner = FakeRunner::new();
    let outcome = revert(&fx.store, &fresh_runner, &key("PROJ-1"), "web").unwrap();
    assert!(matches!(outcome, RevertOutcome::Reverted { .. }));

    assert_eq!(fx.web.hash("composer.json"), json_hash);
    assert_eq!(fx.web.hash("composer.lock"), lock_hash);
    assert_eq!(fresh_runner.log(), ["web: composer install"]);
    assert!(overlays::list_all(&fx.store).unwrap().is_empty());
}

#[test]
fn a_failed_update_leaves_the_overlay_recorded_so_it_can_be_reverted() {
    let fx = fixture();
    let (json_hash, lock_hash) = (fx.web.hash("composer.json"), fx.web.hash("composer.lock"));
    fx.runner.fail_on("composer update");

    let err = apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap_err();
    assert!(format!("{err:#}").contains("composer update"), "{err:#}");

    // composer.json is already edited, but the original is safe in the database.
    assert_ne!(fx.web.hash("composer.json"), json_hash);
    assert!(
        overlays::get(&fx.store, &key("PROJ-1"), "web")
            .unwrap()
            .is_some()
    );

    fx.runner.stop_failing();
    revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(fx.web.hash("composer.json"), json_hash);
    assert_eq!(fx.web.hash("composer.lock"), lock_hash);
}

#[test]
fn a_failed_rebuild_step_fails_the_apply() {
    let fx = fixture();
    fx.runner.fail_on("task build-ui");
    assert!(apply_web(&fx, &fx.runner, &key("PROJ-1")).is_err());
    assert!(
        overlays::get(&fx.store, &key("PROJ-1"), "web")
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_failed_install_keeps_the_backup_and_a_retry_does_not_overwrite_later_edits() {
    let fx = fixture();
    let json_hash = fx.web.hash("composer.json");
    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();

    fx.runner.fail_on("composer install");
    let err = revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap_err();
    assert!(format!("{err:#}").contains("vendor/"), "{err:#}");

    // The files are already back; the backup stays so the install can be retried.
    assert_eq!(fx.web.hash("composer.json"), json_hash);
    let kept = overlays::get(&fx.store, &key("PROJ-1"), "web")
        .unwrap()
        .unwrap();
    assert!(kept.files_restored);

    // A legitimate edit made in the meantime must survive the retry.
    fx.web.write("composer.json", "{\"edited\": \"by hand\"}\n");
    fx.runner.stop_failing();
    revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(fx.web.read("composer.json"), "{\"edited\": \"by hand\"}\n");
    assert!(overlays::list_all(&fx.store).unwrap().is_empty());
}

#[test]
fn the_real_process_runner_works_with_a_stub_composer_executable() {
    let fx = fixture();
    let bin = tempfile::tempdir().unwrap();
    let script = bin.path().join("composer");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         echo \"composer $*\" >> build.log\n\
         case \"$1\" in\n\
           update) echo '{\"stub\": \"lock\"}' > composer.lock ;;\n\
           install) [ -f composer.lock ] || echo '{\"stub\": \"created\"}' > composer.lock ;;\n\
         esac\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let runner = ProcessRunner::new().with_path_prefix(bin.path());

    let (json_hash, lock_hash) = (fx.web.hash("composer.json"), fx.web.hash("composer.lock"));
    let rebuild = [ExternalCommand::new(
        &fx.web.dir,
        "sh",
        &["-c", "echo rebuilt >> build.log"],
        "task build-ui",
    )];
    apply(
        &fx.store,
        &runner,
        &ApplyRequest {
            ticket: &key("PROJ-1"),
            repo: "web",
            consumer_dir: &fx.web.dir,
            packages: &packages(&fx),
            rebuild: &rebuild,
        },
        1,
    )
    .unwrap();
    assert_eq!(fx.web.read("composer.lock"), "{\"stub\": \"lock\"}\n");

    revert(&fx.store, &runner, &key("PROJ-1"), "web").unwrap();
    assert_eq!(fx.web.hash("composer.json"), json_hash);
    assert_eq!(fx.web.hash("composer.lock"), lock_hash);
    assert_eq!(
        fx.web.read("build.log"),
        "composer update acme/api-client\nrebuilt\ncomposer install\n"
    );
}

// ------------------------------------------------------------------ guard

const PACKAGES: &[&str] = &["acme/api-client"];

/// Overlay `web`'s composer files the way the engine would, and commit that.
fn commit_overlay(web: &Repo, message: &str) -> String {
    let edited = composer::apply_overlay(
        web.read("composer.json").as_bytes(),
        &[("acme/api-client".into(), "../api-client".into())],
    )
    .unwrap();
    web.write("composer.json", &String::from_utf8(edited).unwrap());
    web.write(
        "composer.lock",
        "{\n    \"packages\": [\n        {\n            \"name\": \"acme/api-client\",\n            \"dist\": {\n                \"type\": \"path\",\n                \"url\": \"../api-client\"\n            }\n        }\n    ]\n}\n",
    );
    web.commit_all(message)
}

#[test]
fn a_clean_history_passes_the_push_guard() {
    let fx = Fixture::new();
    let web = &fx.web;
    web.git(&["switch", "-c", "feature/PROJ-1-web"]);
    web.commit("app.php", "<?php // changed\n", "code change");
    // A legitimate composer change: bump a version, add a package. Not an overlay.
    web.write("composer.json", &WEB_COMPOSER_JSON.replace("^3.0", "^3.1"));
    web.write(
        "composer.lock",
        &WEB_COMPOSER_LOCK.replace("2.1.0", "2.1.1"),
    );
    web.commit_all("bump monolog");

    check_range_for_overlay(&web.open(), "develop", "HEAD", PACKAGES).unwrap();
}

#[test]
fn an_overlay_leaked_in_a_middle_commit_is_found_with_commit_and_file() {
    let fx = Fixture::new();
    let web = &fx.web;
    web.git(&["switch", "-c", "feature/PROJ-1-web"]);
    web.commit("a.txt", "one\n", "first");
    let leaking = commit_overlay(web, "oops, committed the overlay");
    web.commit("b.txt", "two\n", "after");
    // Later cleaned up: the head tree is fine, but the leaking commit would still be pushed.
    web.git(&[
        "checkout",
        "develop",
        "--",
        "composer.json",
        "composer.lock",
    ]);
    web.commit_all("remove the overlay again");
    assert!(!working_tree_has_overlay(&web.dir, PACKAGES).unwrap());

    let err = check_range_for_overlay(&web.open(), "develop", "HEAD", PACKAGES).unwrap_err();
    let OverlayGuardError::Leak(leak) = err else {
        panic!("expected a leak, got {err}");
    };

    assert!(leak.leaks.iter().all(|l| l.commit == leaking), "{leak}");
    assert!(
        leak.leaks
            .iter()
            .all(|l| l.summary == "oops, committed the overlay")
    );
    let found =
        |file: &str, kind: LeakKind| leak.leaks.iter().any(|l| l.file == file && l.kind == kind);
    assert!(found("composer.json", LeakKind::PathRepository));
    assert!(found("composer.json", LeakKind::WildcardConstraint));
    assert!(found("composer.lock", LeakKind::LockPathDist));
    assert!(leak.leaks.iter().all(|l| l.line.is_some()));

    let text = leak.to_string();
    assert!(
        text.contains("composer.json") && text.contains("oops, committed the overlay"),
        "{text}"
    );
}

#[test]
fn only_the_star_constraint_is_a_leak_for_a_mapped_package() {
    let fx = Fixture::new();
    let web = &fx.web;
    web.git(&["switch", "-c", "feature/x"]);
    web.write(
        "composer.json",
        &WEB_COMPOSER_JSON.replace("\"^3.0\"", "\"*\""),
    );
    web.commit_all("wildcard monolog");

    // monolog is not an overlay package, so a "*" for it is not the overlay.
    check_range_for_overlay(&web.open(), "develop", "HEAD", PACKAGES).unwrap();
    // ... but it is when the config says it is.
    let err =
        check_range_for_overlay(&web.open(), "develop", "HEAD", &["monolog/monolog"]).unwrap_err();
    assert!(
        matches!(&err, OverlayGuardError::Leak(l) if l.leaks[0].kind == LeakKind::WildcardConstraint)
    );
}

#[test]
fn removals_and_the_base_side_are_not_leaks() {
    let fx = Fixture::new();
    let web = &fx.web;
    // The overlay is already in the base (someone leaked it earlier); a new clean commit is fine.
    commit_overlay(web, "leaked long ago");
    web.git(&["switch", "-c", "feature/y"]);
    web.commit("c.txt", "clean\n", "clean commit");
    let base = web.git(&["rev-parse", "develop"]);

    check_range_for_overlay(&web.open(), &base, "HEAD", PACKAGES).unwrap();
}

#[test]
fn merge_commits_do_not_hide_or_duplicate_findings() {
    let fx = Fixture::new();
    let web = &fx.web;
    web.git(&["switch", "-c", "feature/z"]);
    web.commit("z.txt", "z\n", "feature work");
    web.git(&["switch", "develop"]);
    web.commit("d.txt", "d\n", "develop work");
    let base = web.git(&["rev-parse", "develop"]);
    web.git(&["switch", "feature/z"]);
    web.git(&["merge", "--no-ff", "-m", "merge develop", "develop"]);

    check_range_for_overlay(&web.open(), &base, "HEAD", PACKAGES).unwrap();
}

#[test]
fn an_unreadable_range_fails_closed() {
    let fx = Fixture::new();
    let err =
        check_range_for_overlay(&fx.web.open(), "no-such-branch", "HEAD", PACKAGES).unwrap_err();
    assert!(matches!(err, OverlayGuardError::Unreadable(_)));
}

#[test]
fn the_working_tree_check_sees_an_applied_overlay_and_only_that() {
    let fx = fixture();
    assert!(!working_tree_has_overlay(&fx.web.dir, PACKAGES).unwrap());
    assert!(
        working_tree_overlay(&fx.web.dir, PACKAGES)
            .unwrap()
            .is_empty()
    );

    apply_web(&fx, &fx.runner, &key("PROJ-1")).unwrap();
    assert!(working_tree_has_overlay(&fx.web.dir, PACKAGES).unwrap());
    let kinds: Vec<LeakKind> = working_tree_overlay(&fx.web.dir, PACKAGES)
        .unwrap()
        .into_iter()
        .map(|f| f.kind)
        .collect();
    assert!(kinds.contains(&LeakKind::PathRepository));
    assert!(kinds.contains(&LeakKind::WildcardConstraint));

    revert(&fx.store, &fx.runner, &key("PROJ-1"), "web").unwrap();
    assert!(!working_tree_has_overlay(&fx.web.dir, PACKAGES).unwrap());

    // No composer files at all is fine.
    assert!(!working_tree_has_overlay(&fx.docs.dir, PACKAGES).unwrap());
}

//! Integration tests: real git repositories with bare origins, a real `state.db`, and the
//! provider fakes; nothing about git or SQLite is mocked.

use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::{
    activation::{ActivateOptions, WorkspaceRepo, activate},
    config::Config,
    domain::{LocalStatus, TicketKey},
    gateway::{Action, Gateway, PushReport, PushResult},
    git::testutil::git,
    providers::{
        PipelineRun, PipelineState, PipelineStep, ProviderError,
        fake::{FakeJira, build::ticket},
    },
    store::{
        Kind, Store, audit,
        drafts::{self, DraftStatus},
        links, overlays, pipelines, prs, restore, tickets, uat_details,
        uat_details::{MergeDetails, MergeKind},
        uat_merges::{self, UatMerge},
    },
    sync::HostedRepo,
    testsupport::{Fixture, Repo, WEB_COMPOSER_JSON, key},
};

const LEAK: &str = "{\"repositories\": [{\"type\": \"path\", \"url\": \"../api-client\", \"options\": {\"symlink\": true}}]}\n";

struct Env {
    fx: Fixture,
    data: TempDir,
    ticket: TicketKey,
}

impl Env {
    /// PROJ-1 has a branch in `api-client` and in `worker`; both have `uat` on their origin.
    fn new() -> Self {
        let fx = Fixture::with_ticket();
        fx.worker.add_branch("feature/PROJ-1-work", "w.txt", false);
        fx.api_client.add_uat();
        fx.worker.add_uat();
        Self {
            fx,
            data: TempDir::new().unwrap(),
            ticket: key("PROJ-1"),
        }
    }

    fn activate(&self) {
        activate(
            &self.fx.store,
            &self.fx.runner,
            &self.ticket,
            &self.fx.repos(),
            &ActivateOptions::default(),
            10,
        )
        .unwrap();
    }

    fn active() -> Self {
        let env = Self::new();
        env.activate();
        env
    }

    fn prepare(&self) -> IntegrationPrep {
        self.prepare_with(&self.fx.repos())
    }

    fn prepare_with(&self, repos: &[WorkspaceRepo]) -> IntegrationPrep {
        prepare_integration(
            &self.fx.store,
            &self.fx.runner,
            self.data.path(),
            &self.ticket,
            repos,
            20,
        )
        .unwrap()
    }

    fn gateway(&self) -> Gateway<'_> {
        let missing = || ProviderError::not_installed("acli", "adapter not available");
        Gateway::with_writers(&self.fx.store, Err(missing()), Err(missing()))
    }

    fn push(&self, prep: &IntegrationPrep) -> PushReport {
        let gw = self.gateway();
        let draft = gw.draft(prep.push_action().unwrap());
        execute_push(&gw, draft.confirm(), 30).unwrap().0
    }

    fn repo(&self, name: &str) -> &Repo {
        self.fx.all().into_iter().find(|r| r.name == name).unwrap()
    }

    fn outcome<'a>(&self, prep: &'a IntegrationPrep, name: &str) -> &'a RepoOutcome {
        &prep.repos.iter().find(|r| r.repo == name).unwrap().outcome
    }

    fn merges(&self) -> Vec<(String, String)> {
        uat_merges::list_for_ticket(&self.fx.store, &self.ticket)
            .unwrap()
            .into_iter()
            .map(|m| (m.repo, m.commit))
            .collect()
    }

    fn origin_uats(&self) -> Vec<String> {
        ["api-client", "worker"]
            .iter()
            .map(|n| self.repo(n).origin_sha("uat"))
            .collect()
    }

    fn status(&self) -> LocalStatus {
        tickets::get(&self.fx.store, &self.ticket)
            .unwrap()
            .unwrap()
            .status
    }
}

fn ready(outcome: &RepoOutcome) -> &ReadyMerge {
    match outcome {
        RepoOutcome::Ready(r) => r,
        other => panic!("expected Ready, got {other:?}"),
    }
}

fn blocked(outcome: &RepoOutcome) -> &str {
    match outcome {
        RepoOutcome::Blocked { reason } => reason,
        other => panic!("expected Blocked, got {other:?}"),
    }
}

fn is_clean(dir: &std::path::Path) -> bool {
    crate::git::GitRepo::open(dir)
        .unwrap()
        .status()
        .unwrap()
        .is_clean()
}

fn worktree_count(repo: &Repo) -> usize {
    repo.git(&["worktree", "list"]).lines().count()
}

// --------------------------------------------------------------- prepare

#[test]
fn a_clean_merge_is_ready_and_pushes_nothing_and_disturbs_nothing() {
    let env = Env::new();
    let before = env.fx.snapshot();
    env.activate();
    let active_snapshot = env.fx.snapshot();
    let uats = env.origin_uats();
    let web_json = env.fx.web.read("composer.json");

    let prep = env.prepare();

    assert!(prep.is_ready());
    assert_eq!(
        prep.repos.len(),
        2,
        "only the repos with the ticket branch, not baselines"
    );
    let names: Vec<_> = prep.repos.iter().map(|r| r.repo.as_str()).collect();
    assert!(names.contains(&"api-client") && names.contains(&"worker"));

    for name in ["api-client", "worker"] {
        let repo = env.repo(name);
        let entry = prep.repos.iter().find(|r| r.repo == name).unwrap();
        let merge = ready(&entry.outcome);
        assert_eq!(merge.uat_before, repo.origin_sha("uat"));
        assert_eq!(merge.commits.len(), 1);
        assert_eq!(merge.files.len(), 1, "{:?}", merge.files);

        // The merge commit is a --no-ff merge of the ticket branch tip into uat.
        let parents = repo.git(&["rev-list", "--parents", "-n1", &merge.merge_commit]);
        let tip = repo.git(&["rev-parse", &entry.ticket_branch]);
        assert_eq!(
            repo.git(&["log", "-1", "--format=%s", &merge.merge_commit]),
            format!("Merge branch '{}' into uat", entry.ticket_branch)
        );
        assert_eq!(
            parents,
            format!("{} {} {}", merge.merge_commit, merge.uat_before, tip)
        );
        assert_eq!(entry.ticket_tip.as_deref(), Some(tip.as_str()));

        // In a temporary worktree under the data dir, not the user's checkout.
        let wt = entry.worktree.clone().unwrap();
        assert_eq!(wt, integration_dir(env.data.path(), &env.ticket).join(name));
        assert!(wt.starts_with(env.data.path()) && is_clean(&wt));
        assert_eq!(worktree_count(repo), 2);
    }

    // Nothing pushed, nothing recorded, the test checkout is exactly as it was.
    assert_eq!(env.origin_uats(), uats);
    assert!(env.merges().is_empty());
    assert_eq!(env.fx.snapshot(), active_snapshot);
    assert_eq!(
        env.fx.web.read("composer.json"),
        web_json,
        "the overlay stays for testing"
    );
    assert!(
        overlays::find_by_dir(&env.fx.store, &env.fx.web.dir)
            .unwrap()
            .is_some()
    );
    assert_ne!(active_snapshot, before);
    assert_eq!(env.status(), LocalStatus::Active);
    assert!(env.fx.api_client.is_clean() && env.fx.worker.is_clean());
}

#[test]
fn a_ticket_that_is_not_active_or_touches_nothing_is_refused() {
    let env = Env::new();
    let err = prepare_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &env.ticket,
        &env.fx.repos(),
        1,
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("not active"), "{err:#}");

    env.activate();
    restore::list(&env.fx.store, &env.ticket)
        .unwrap()
        .into_iter()
        .for_each(|r| {
            restore::delete(&env.fx.store, &env.ticket, &r.repo).unwrap();
        });
    let err = prepare_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &env.ticket,
        &env.fx.repos(),
        1,
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("nothing to integrate"),
        "{err:#}"
    );
}

#[test]
fn a_ticket_already_in_uat_is_up_to_date_and_there_is_nothing_to_push() {
    let env = Env::new();
    for name in ["api-client", "worker"] {
        let repo = env.repo(name);
        let branch = if name == "worker" {
            "feature/PROJ-1-work"
        } else {
            "feature/PROJ-1-api-change"
        };
        repo.git(&["push", "origin", &format!("{branch}:refs/heads/uat")]);
    }
    env.activate();
    let prep = env.prepare();
    for name in ["api-client", "worker"] {
        assert!(matches!(
            env.outcome(&prep, name),
            RepoOutcome::UpToDate { .. }
        ));
    }
    assert!(!prep.is_ready());
    assert!(
        prep.push_action()
            .unwrap_err()
            .to_string()
            .contains("Nothing to push")
    );
    // No worktree was needed.
    assert!(prep.repos.iter().all(|r| r.worktree.is_none()));

    // Finalize records "already in uat" so deploy tracking can follow the uat tip.
    let report = finalize_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &prep,
        &env.fx.repos(),
        40,
    )
    .unwrap();
    assert_eq!(report.deactivation.status, Some(LocalStatus::Integrated));
    assert_eq!(report.already_in_uat.len(), 2);
    let recorded = uat_details::list_for_ticket(&env.fx.store, &env.ticket).unwrap();
    assert!(
        recorded
            .iter()
            .all(|m| m.details.as_ref().unwrap().kind == MergeKind::AlreadyInUat)
    );
    assert_eq!(
        recorded[0].merge.commit,
        env.repo(&recorded[0].merge.repo).origin_sha("uat")
    );
}

#[test]
fn a_conflict_is_reported_with_files_and_leaves_everything_clean() {
    let env = Env::new();
    env.repo("api-client")
        .advance_origin("uat", "src/Client.php");
    env.activate();
    let snapshot = env.fx.snapshot();
    let uats = env.origin_uats();

    let prep = env.prepare();

    match env.outcome(&prep, "api-client") {
        RepoOutcome::Conflict { files } => assert_eq!(files, &["src/Client.php"]),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        env.outcome(&prep, "worker"),
        RepoOutcome::Ready(_)
    ));
    let wt = prep
        .repos
        .iter()
        .find(|r| r.repo == "api-client")
        .unwrap()
        .worktree
        .clone()
        .unwrap();
    assert!(is_clean(&wt), "the merge was aborted");
    assert_eq!(
        git(&wt, &["rev-parse", "HEAD"]),
        env.repo("api-client").origin_sha("uat"),
        "the worktree is back on uat"
    );
    assert!(!prep.is_ready() && !prep.is_pushable());
    let err = prep.push_action().unwrap_err().to_string();
    assert!(
        err.contains("conflicts") && err.contains("src/Client.php"),
        "{err}"
    );
    assert_eq!(env.fx.snapshot(), snapshot);
    assert!(env.fx.api_client.is_clean());
    assert_eq!(env.origin_uats()[1], uats[1]);
    assert!(env.merges().is_empty());
}

#[test]
fn a_local_uat_with_commits_the_remote_lacks_is_refused() {
    let env = Env::new();
    let api = env.repo("api-client");
    api.git(&["switch", "-c", "uat", "origin/uat"]);
    api.commit("local-only.txt", "x\n", "local uat commit");
    api.git(&["switch", "develop"]);
    env.activate();

    let prep = env.prepare();
    let reason = blocked(env.outcome(&prep, "api-client"));
    assert!(reason.contains("local uat has 1 commit"), "{reason}");
    assert!(prep.push_action().is_err());
    // The user's local uat was not moved.
    assert_eq!(
        api.git(&["log", "-1", "--format=%s", "uat"]),
        "local uat commit"
    );
}

#[test]
fn a_local_uat_that_is_merely_behind_is_left_alone() {
    let env = Env::new();
    let api = env.repo("api-client");
    api.git(&["branch", "uat", "origin/uat"]);
    let old = api.git(&["rev-parse", "uat"]);
    api.advance_origin("uat", "later.txt");
    env.activate();
    let prep = env.prepare();
    let merge = ready(env.outcome(&prep, "api-client"));
    assert_eq!(
        merge.uat_before,
        api.origin_sha("uat"),
        "merged onto the fetched remote uat"
    );
    assert_eq!(api.git(&["rev-parse", "uat"]), old);
}

#[test]
fn a_ticket_branch_with_the_overlay_is_blocked_by_sha_even_with_colliding_tag_names() {
    let env = Env::new();
    let api = env.repo("api-client");
    // The leak is committed on the ticket branch.
    api.git(&["switch", "feature/PROJ-1-api-change"]);
    api.commit("composer.json", LEAK, "oops: overlay committed");
    api.git(&["push", "origin", "feature/PROJ-1-api-change"]);
    api.git(&["switch", "develop"]);
    env.activate();

    // A tag with the branch's name pointing at clean history, and a tag named `uat`.
    let clean = api.git(&["rev-parse", "develop"]);
    api.git(&["tag", "feature/PROJ-1-api-change", &clean]);
    api.git(&["tag", "uat", &clean]);

    let prep = env.prepare();
    let reason = blocked(env.outcome(&prep, "api-client"));
    assert!(
        reason.contains("test overlay") && reason.contains("composer.json"),
        "{reason}"
    );
    let entry = prep.repos.iter().find(|r| r.repo == "api-client").unwrap();
    assert_eq!(
        entry.ticket_tip.as_deref(),
        Some(
            api.git(&["rev-parse", "refs/heads/feature/PROJ-1-api-change"])
                .as_str()
        ),
        "the branch, not the tag"
    );
    assert!(prep.push_action().is_err());
    assert_eq!(env.origin_uats()[0], api.origin_sha("uat"));
}

#[test]
fn tags_named_like_the_branches_do_not_change_what_is_merged() {
    let env = Env::new();
    let api = env.repo("api-client");
    env.activate();
    let tip = api.git(&["rev-parse", "refs/heads/feature/PROJ-1-api-change"]);
    let origin_uat = api.origin_sha("uat");
    // Tags win over branches for bare names; the flow must not use bare names.
    api.git(&["tag", "feature/PROJ-1-api-change", &origin_uat]);
    api.git(&["tag", "uat", &tip]);

    let prep = env.prepare();
    let merge = ready(env.outcome(&prep, "api-client"));
    assert_eq!(merge.uat_before, origin_uat);
    let parents = api.git(&["rev-list", "--parents", "-n1", &merge.merge_commit]);
    assert!(parents.ends_with(&tip), "{parents}");
}

#[test]
fn configured_checks_run_in_the_worktree_and_a_failure_blocks_that_repo() {
    let env = Env::active();
    let mut repos = env.fx.repos();
    let worker = repos.iter_mut().find(|r| r.name == "worker").unwrap();
    worker.manifest = toml::from_str(
        "[project]\nname = \"worker\"\nworkspace = \"shop\"\n[tasks]\nlint = \"make lint\"\n[integrate]\nchecks = [\"lint\"]\n",
    )
    .unwrap();

    let prep = env.prepare_with(&repos);
    assert!(matches!(
        env.outcome(&prep, "worker"),
        RepoOutcome::Ready(_)
    ));
    let calls = env.fx.runner.calls();
    let check = calls
        .iter()
        .find(|c| c.label == "check lint")
        .expect("the check ran");
    let wt = integration_dir(env.data.path(), &env.ticket).join("worker");
    assert_eq!(check.dir, wt, "run in the integration worktree");
    assert!(!calls.iter().any(|c| c.label == "check lint" && c.dir != wt));

    env.fx.runner.fail_on("check lint");
    let prep = env.prepare_with(&repos);
    let reason = blocked(env.outcome(&prep, "worker"));
    assert!(
        reason.contains("check 'lint' failed") && reason.contains("boom"),
        "{reason}"
    );
    assert!(matches!(
        env.outcome(&prep, "api-client"),
        RepoOutcome::Ready(_)
    ));
    assert!(
        prep.push_action().is_err(),
        "nothing is offered while a repo is blocked"
    );

    // A check that names an undefined task blocks too.
    let worker = repos.iter_mut().find(|r| r.name == "worker").unwrap();
    worker.manifest.integrate.checks = vec!["nope".into()];
    let prep = env.prepare_with(&repos);
    assert!(blocked(env.outcome(&prep, "worker")).contains("'nope' is not defined"));
}

#[test]
fn preparing_again_replaces_stale_worktrees_and_cancel_removes_them() {
    let env = Env::active();
    let first = env.prepare();
    let second = env.prepare();
    assert!(second.is_ready());
    assert_ne!(ready(env.outcome(&first, "worker")).merge_commit.len(), 0);
    assert_eq!(worktree_count(&env.fx.worker), 2, "one worktree, not two");

    let removed = cancel_integration(
        &env.fx.store,
        env.data.path(),
        &env.ticket,
        &env.fx.repos(),
        50,
    )
    .unwrap();
    assert_eq!(removed.len(), 2);
    assert!(!integration_dir(env.data.path(), &env.ticket).exists());
    for name in ["api-client", "worker"] {
        let repo = env.repo(name);
        assert_eq!(worktree_count(repo), 1);
        assert_eq!(repo.git(&["branch", "--list", "de/integrate/*"]), "");
    }
    // Cancelling again is harmless, and prepare works after a cancel.
    cancel_integration(
        &env.fx.store,
        env.data.path(),
        &env.ticket,
        &env.fx.repos(),
        51,
    )
    .unwrap();
    assert!(env.prepare().is_ready());
}

// ------------------------------------------------------------------ push

#[test]
fn pushing_records_each_merge_and_finalize_makes_the_ticket_integrated() {
    let env = Env::new();
    let before = env.fx.snapshot();
    env.activate();
    let prep = env.prepare();
    let expected: Vec<(String, String)> = prep
        .repos
        .iter()
        .map(|r| (r.repo.clone(), ready(&r.outcome).merge_commit.clone()))
        .collect();

    let report = env.push(&prep);
    assert!(report.all_done(), "{report:?}");
    assert_eq!(env.merges(), expected, "recorded in push order");
    for (repo, commit) in &expected {
        assert_eq!(
            &env.repo(repo).origin_sha("uat"),
            commit,
            "origin uat is the merge commit"
        );
        let details = uat_details::latest(&env.fx.store, &env.ticket, repo)
            .unwrap()
            .unwrap();
        assert_eq!(details.details.unwrap().kind, MergeKind::Merge);
    }
    // Pushed, but not integrated until finalize.
    assert_eq!(env.status(), LocalStatus::Active);

    // The audit log has the attempt with the payload, then the outcome and a row per repo.
    let rows: Vec<String> = audit::list(&env.fx.store, Some(&env.ticket), 100)
        .unwrap()
        .into_iter()
        .rev()
        .map(|e| e.action)
        .filter(|a| a.starts_with("git."))
        .collect();
    assert_eq!(
        rows,
        [
            "git.push_uat.attempted",
            "git.push_uat.repo",
            "git.push_uat.repo",
            "git.push_uat"
        ]
    );

    let finalize = finalize_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &prep,
        &env.fx.repos(),
        40,
    )
    .unwrap();
    assert!(finalize.deactivation.is_complete());
    assert_eq!(env.status(), LocalStatus::Integrated);
    // Overlay reverted, branches restored, worktrees gone.
    assert_eq!(env.fx.web.read("composer.json"), WEB_COMPOSER_JSON);
    assert!(overlays::list_all(&env.fx.store).unwrap().is_empty());
    assert!(restore::list_all(&env.fx.store).unwrap().is_empty());
    assert_eq!(env.fx.snapshot(), before);
    assert!(!integration_dir(env.data.path(), &env.ticket).exists());
    assert_eq!(worktree_count(&env.fx.worker), 1);
}

#[test]
fn finalize_refuses_until_every_repo_is_pushed_and_integrated_has_no_other_door() {
    let env = Env::active();
    let prep = env.prepare();
    let err = finalize_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &prep,
        &env.fx.repos(),
        40,
    )
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("has not been pushed"),
        "{err:#}"
    );
    assert_eq!(env.status(), LocalStatus::Active);

    // Not by the public status API or the public deactivate either.
    assert!(tickets::set_status(&env.fx.store, &env.ticket, LocalStatus::Integrated, 1).is_err());
    assert!(
        crate::activation::deactivate(
            &env.fx.store,
            &env.fx.runner,
            &env.ticket,
            LocalStatus::Integrated,
            1
        )
        .is_err()
    );
    assert_eq!(env.status(), LocalStatus::Active);

    // A blocked prep cannot be finalized either.
    env.repo("api-client")
        .advance_origin("uat", "src/Client.php");
    let conflicting = env.prepare();
    assert!(
        finalize_integration(
            &env.fx.store,
            &env.fx.runner,
            env.data.path(),
            &conflicting,
            &env.fx.repos(),
            41,
        )
        .is_err()
    );
    assert_eq!(env.status(), LocalStatus::Active);
}

#[test]
fn a_push_is_rejected_when_uat_moved_after_prepare_and_nothing_is_forced_or_recorded() {
    let env = Env::active();
    let prep = env.prepare();
    let first = prep.repos[0].repo.clone();
    let moved = env.repo(&first).advance_origin("uat", "someone-else.txt");

    let report = env.push(&prep);
    assert!(
        matches!(report.repos[0].result, PushResult::Rejected { .. }),
        "{report:?}"
    );
    assert_eq!(report.repos[1].result, PushResult::NotAttempted);
    assert_eq!(
        env.repo(&first).origin_sha("uat"),
        moved,
        "the other clone's push stands"
    );
    assert!(env.merges().is_empty());
    assert_eq!(env.status(), LocalStatus::Active);

    // Retrying the same stale preparation is safe: still rejected, still nothing forced.
    let again = env.push(&prep);
    assert!(matches!(again.repos[0].result, PushResult::Rejected { .. }));
    assert_eq!(env.repo(&first).origin_sha("uat"), moved);

    // The rejection is recorded as a failure in the audit log.
    let last_repo = audit::list(&env.fx.store, Some(&env.ticket), 100)
        .unwrap()
        .into_iter()
        .find(|e| e.action == "git.push_uat.repo")
        .unwrap();
    assert_eq!(last_repo.outcome, crate::domain::AuditOutcome::Failure);
}

#[test]
fn a_partial_failure_keeps_the_first_repo_recorded_and_a_rerun_completes_the_rest() {
    let env = Env::active();
    let prep = env.prepare();
    let (first, second) = (prep.repos[0].repo.clone(), prep.repos[1].repo.clone());
    let first_merge = ready(&prep.repos[0].outcome).merge_commit.clone();
    let moved = env.repo(&second).advance_origin("uat", "someone-else.txt");

    let report = env.push(&prep);
    assert_eq!(
        report.repos[0].result,
        PushResult::Pushed {
            merge_commit: first_merge.clone()
        }
    );
    assert!(
        matches!(report.repos[1].result, PushResult::Rejected { .. }),
        "{report:?}"
    );
    assert!(!report.all_done());

    // The first is recorded right away; the ticket stays Active.
    assert_eq!(env.merges(), [(first.clone(), first_merge.clone())]);
    assert_eq!(env.status(), LocalStatus::Active);
    assert_eq!(env.repo(&first).origin_sha("uat"), first_merge);
    assert_eq!(env.repo(&second).origin_sha("uat"), moved);
    assert!(
        finalize_integration(
            &env.fx.store,
            &env.fx.runner,
            env.data.path(),
            &prep,
            &env.fx.repos(),
            40
        )
        .is_err()
    );

    // Re-run: the pushed repo is skipped, the other is merged onto the new uat.
    let prep = env.prepare();
    assert_eq!(
        env.outcome(&prep, &first),
        &RepoOutcome::AlreadyPushed {
            merge_commit: first_merge.clone()
        }
    );
    let second_merge = ready(env.outcome(&prep, &second));
    assert_eq!(second_merge.uat_before, moved);
    let Action::PushUat(push) = prep.push_action().unwrap() else {
        panic!()
    };
    assert_eq!(
        push.repos.len(),
        1,
        "only the repo that still needs pushing"
    );

    let report = env.push(&prep);
    assert!(report.all_done(), "{report:?}");
    assert_eq!(env.merges().len(), 2);
    assert_eq!(
        env.repo(&first).origin_sha("uat"),
        first_merge,
        "the first repo was not pushed again"
    );

    finalize_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &prep,
        &env.fx.repos(),
        41,
    )
    .unwrap();
    assert_eq!(env.status(), LocalStatus::Integrated);
}

#[test]
fn the_gateway_rechecks_the_merge_commit_before_pushing() {
    let env = Env::active();
    let prep = env.prepare();
    let Action::PushUat(mut push) = prep.push_action().unwrap() else {
        panic!()
    };
    // A commit that is not the prepared merge of the tip into uat.
    push.repos[0].merge_commit = push.repos[0].ticket_tip.clone();
    let uats = env.origin_uats();

    let gw = env.gateway();
    let (report, _) = execute_push(&gw, gw.draft(Action::PushUat(push)).confirm(), 30).unwrap();
    assert!(
        matches!(report.repos[0].result, PushResult::Blocked { .. }),
        "{report:?}"
    );
    assert!(report.repos.iter().all(|r| !r.result.is_done()));
    assert_eq!(env.origin_uats(), uats);
    assert!(env.merges().is_empty());
}

#[test]
fn a_push_of_an_inactive_ticket_is_refused_by_the_gateway() {
    let env = Env::active();
    let prep = env.prepare();
    let action = prep.push_action().unwrap();
    crate::activation::park(&env.fx.store, &env.fx.runner, &env.ticket, 25).unwrap();
    let gw = env.gateway();
    let err = execute_push(&gw, gw.draft(action).confirm(), 30).unwrap_err();
    assert!(err.to_string().contains("not active"), "{err}");
}

#[test]
fn a_recorded_merge_that_uat_no_longer_contains_is_pushed_again() {
    // uat is long-lived but teams do reset it. A merge recorded earlier must not make the
    // ticket count as "already pushed" when the remote uat no longer contains it.
    let env = Env::active();
    let prep = env.prepare();
    assert!(env.push(&prep).all_done());
    let first = prep.repos[0].repo.clone();
    let before = ready(&prep.repos[0].outcome).uat_before.clone();
    env.repo(&first).git(&[
        "push",
        "--force",
        "origin",
        &format!("{before}:refs/heads/uat"),
    ]);

    let again = env.prepare();
    assert!(
        matches!(env.outcome(&again, &first), RepoOutcome::Ready(_)),
        "{:?}",
        env.outcome(&again, &first)
    );
    // The repo whose uat still has the merge stays "already pushed".
    let second = prep.repos[1].repo.clone();
    assert!(matches!(
        env.outcome(&again, &second),
        RepoOutcome::AlreadyPushed { .. }
    ));

    // And it really is pushed again (the gateway does not skip it on the stale record), and
    // the ticket can then be finalized.
    let new_merge = ready(env.outcome(&again, &first)).merge_commit.clone();
    assert!(env.push(&again).all_done());
    assert_eq!(env.repo(&first).origin_sha("uat"), new_merge);
    finalize_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &again,
        &env.fx.repos(),
        60,
    )
    .unwrap();
    assert_eq!(env.status(), LocalStatus::Integrated);
}

#[cfg(unix)]
#[test]
fn hooks_of_the_repo_do_not_stop_the_merge_commit_from_being_worded() {
    use std::os::unix::fs::PermissionsExt;

    let env = Env::active();
    for name in ["api-client", "worker"] {
        let hook = env.repo(name).dir.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prep = env.prepare();
    assert!(
        prep.is_ready(),
        "a failing pre-commit hook blocked the integration: {:?}",
        prep.repos.iter().map(|r| &r.outcome).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------- deploy status

struct Deploys {
    env: Env,
    cache: Store,
    hosted: Vec<HostedRepo>,
    base: String,
}

impl Deploys {
    fn new() -> Self {
        let env = Env::new();
        let api = env.repo("api-client");
        let hosted = vec![HostedRepo {
            project: "api-client".into(),
            repo: "acme/api-client".into(),
            dir: api.dir.clone(),
            branches: Default::default(),
            deploy_environment: Some("alpha".into()),
        }];
        Self {
            base: api.head(),
            cache: Store::open_in_memory(Kind::Cache).unwrap(),
            hosted,
            env,
        }
    }

    /// A ticket whose recorded merge is `m`, with a child commit `d` on its own branch.
    fn merged(&self, ticket: &str) -> (TicketKey, String, String) {
        let api = self.env.repo("api-client");
        let k = key(ticket);
        tickets::claim(&self.env.fx.store, &k, 1).unwrap();
        api.git(&["switch", "-c", &format!("chain-{ticket}"), &self.base]);
        let m = api.commit(
            &format!("{ticket}-m.txt"),
            "m\n",
            &format!("{ticket} merge"),
        );
        let d = api.commit(
            &format!("{ticket}-d.txt"),
            "d\n",
            &format!("{ticket} later"),
        );
        api.git(&["switch", "develop"]);
        uat_details::record(
            &self.env.fx.store,
            &UatMerge {
                ticket: k.clone(),
                repo: "api-client".into(),
                branch: "uat".into(),
                commit: m.clone(),
                recorded_at: 5,
            },
            &MergeDetails {
                kind: MergeKind::Merge,
                ticket_branch: format!("feature/{ticket}-x"),
                ticket_tip: m.clone(),
                uat_before: self.base.clone(),
            },
        )
        .unwrap();
        (k, m, d)
    }

    fn run(
        &self,
        id: &str,
        commit: &str,
        state: PipelineState,
        env: Option<(&str, PipelineState)>,
        at: i64,
    ) {
        pipelines::upsert(
            &self.cache,
            &PipelineRun {
                repo: "acme/api-client".into(),
                id: id.into(),
                number: Some(at as u64),
                state,
                branch: "uat".into(),
                commit: commit.into(),
                created_at: at,
                completed_at: None,
                url: format!("https://bb.test/p/{id}"),
                steps: env
                    .into_iter()
                    .map(|(e, s)| PipelineStep {
                        name: "deploy".into(),
                        state: s,
                        deployment_environment: Some(e.into()),
                    })
                    .collect(),
            },
            at,
        )
        .unwrap();
    }

    fn status(&self, k: &TicketKey) -> RepoDeploy {
        let mut all = deploy_status(&self.env.fx.store, &self.cache, k, &self.hosted).unwrap();
        assert_eq!(all.len(), 1);
        all.remove(0)
    }
}

#[test]
fn deploy_status_follows_the_recorded_merge_through_its_pipelines() {
    use PipelineState::*;
    let dp = Deploys::new();
    let alpha = |s| Some(("alpha", s));

    // No push yet: linked, nothing recorded.
    let unpushed = key("PROJ-20");
    tickets::claim(&dp.env.fx.store, &unpushed, 1).unwrap();
    links::add_manual(&dp.env.fx.store, &unpushed, "api-client", Some("b")).unwrap();
    let s = dp.status(&unpushed);
    assert_eq!((s.state, s.merge_commit), (DeployState::NotPushed, None));
    assert!(!all_deployed(&dp.env.fx.store, &dp.cache, &unpushed, &dp.hosted).unwrap());

    // Pushed, no pipeline seen yet.
    let (k, _m, _d) = dp.merged("PROJ-21");
    assert_eq!(dp.status(&k).state, DeployState::Pending);

    // Running.
    let (k, m, _) = dp.merged("PROJ-22");
    dp.run("r22", &m, Running, alpha(Pending), 10);
    let s = dp.status(&k);
    assert_eq!(
        (s.state, s.run.unwrap().id),
        (DeployState::Running, "r22".into())
    );

    // Deployed by the exact-commit run.
    let (k, m, _) = dp.merged("PROJ-23");
    dp.run("r23", &m, Succeeded, alpha(Succeeded), 10);
    let s = dp.status(&k);
    assert_eq!(
        (s.state, s.run.unwrap().id),
        (DeployState::Deployed, "r23".into())
    );
    assert!(all_deployed(&dp.env.fx.store, &dp.cache, &k, &dp.hosted).unwrap());

    // Deployed by a descendant run: another ticket pushed later and its run contains ours.
    let (k, _m, d) = dp.merged("PROJ-24");
    dp.run("r24", &d, Succeeded, alpha(Succeeded), 20);
    let s = dp.status(&k);
    assert_eq!(
        (s.state, s.run.unwrap().id),
        (DeployState::Deployed, "r24".into())
    );

    // The exact-commit run is preferred when both deployed.
    let (k, m, d) = dp.merged("PROJ-25");
    dp.run("r25-later", &d, Succeeded, alpha(Succeeded), 30);
    dp.run("r25-exact", &m, Succeeded, alpha(Succeeded), 20);
    assert_eq!(dp.status(&k).run.unwrap().id, "r25-exact");

    // Failed.
    let (k, m, _) = dp.merged("PROJ-26");
    dp.run("r26", &m, Failed, alpha(Failed), 10);
    assert_eq!(dp.status(&k).state, DeployState::Failed);

    // A failed exact run, but a later run that contains it deployed.
    let (k, m, d) = dp.merged("PROJ-27");
    dp.run("r27a", &m, Failed, alpha(Failed), 10);
    dp.run("r27b", &d, Succeeded, alpha(Succeeded), 20);
    let s = dp.status(&k);
    assert_eq!(
        (s.state, s.run.unwrap().id),
        (DeployState::Deployed, "r27b".into())
    );

    // Deploy environment filter: a deployment to production is not alpha.
    let (k, m, _) = dp.merged("PROJ-28");
    dp.run("r28", &m, Succeeded, Some(("production", Succeeded)), 10);
    assert_eq!(dp.status(&k).state, DeployState::Pending);

    // A stale run of an older commit (an ancestor) never counts, however deployed.
    let (k, _m, _) = dp.merged("PROJ-29");
    dp.run("stale", &dp.base, Succeeded, alpha(Succeeded), 10);
    let s = dp.status(&k);
    assert_eq!((s.state, s.run), (DeployState::Pending, None));

    // A run of an unrelated commit does not count either.
    let (k, _m, other_d) = dp.merged("PROJ-30");
    let (_k2, _m2, d2) = dp.merged("PROJ-31");
    dp.run("unrelated", &d2, Succeeded, alpha(Succeeded), 10);
    assert_eq!(dp.status(&k).state, DeployState::Pending);
    let _ = other_d;
}

#[test]
fn a_repo_without_hosting_cannot_be_followed_and_never_counts_as_deployed() {
    let dp = Deploys::new();
    let (k, _m, _d) = dp.merged("PROJ-40");
    let s = deploy_status(&dp.env.fx.store, &dp.cache, &k, &[])
        .unwrap()
        .remove(0);
    assert_eq!(s.state, DeployState::Untracked);
    assert!(!all_deployed(&dp.env.fx.store, &dp.cache, &k, &[]).unwrap());
}

#[test]
fn needs_remerge_sees_new_ticket_commits_that_uat_lacks() {
    let dp = Deploys::new();
    let env = &dp.env;
    let api = env.repo("api-client");
    let repos = env.fx.repos();
    let repo = repos.iter().find(|r| r.name == "api-client").unwrap();

    // Record the real merge of the ticket branch as the flow would.
    env.activate();
    let prep = env.prepare();
    env.push(&prep);
    finalize_integration(
        &env.fx.store,
        &env.fx.runner,
        env.data.path(),
        &prep,
        &repos,
        40,
    )
    .unwrap();
    let k = &env.ticket;
    assert!(!needs_remerge(&env.fx.store, k, repo).unwrap());
    assert!(
        !needs_remerge(&env.fx.store, &key("PROJ-99"), repo).unwrap(),
        "unknown ticket"
    );

    // A fix lands on the ticket branch locally.
    api.git(&["switch", "feature/PROJ-1-api-change"]);
    let fix = api.commit("fix.txt", "fix\n", "fix after alpha");
    assert!(needs_remerge(&env.fx.store, k, repo).unwrap());

    // ... and once uat contains it, no more.
    api.git(&["switch", "-c", "tmp-uat", "origin/uat"]);
    api.git(&["merge", "--no-ff", "--no-edit", &fix]);
    api.git(&["push", "origin", "tmp-uat:uat"]);
    api.git(&["switch", "feature/PROJ-1-api-change"]);
    assert!(!needs_remerge(&env.fx.store, k, repo).unwrap());

    // A commit that only exists on the remote ticket branch counts too.
    api.git(&["push", "origin", "feature/PROJ-1-api-change"]);
    api.advance_origin("feature/PROJ-1-api-change", "remote-fix.txt");
    api.git(&["fetch", "origin"]);
    assert!(needs_remerge(&env.fx.store, k, repo).unwrap());
}

// ------------------------------------------------------- deploy comment

struct Announce {
    dp: Deploys,
    jira: FakeJira,
    ticket: TicketKey,
    merge: String,
    later: String,
}

impl Announce {
    fn new() -> Self {
        let dp = Deploys::new();
        let (ticket, merge, later) = dp.merged("PROJ-50");
        dp.env
            .repo("api-client")
            .git(&["switch", "-c", "spare", &dp.base]);
        dp.env.repo("api-client").git(&["switch", "develop"]);
        let jira = FakeJira::new();
        jira.add_ticket(ticket_fixture("PROJ-50"));
        Self {
            dp,
            jira,
            ticket,
            merge,
            later,
        }
    }

    fn gateway(&self) -> Gateway<'_> {
        Gateway::with_writers(
            &self.dp.env.fx.store,
            Ok(Box::new(self.jira.clone())),
            Err(ProviderError::not_installed("bkt", "n/a")),
        )
    }

    fn state(&self) -> &Store {
        &self.dp.env.fx.store
    }

    fn compose(&self, partial: bool) -> eyre::Result<drafts::StoredDraft> {
        compose_deploy_comment(
            self.state(),
            &self.dp.cache,
            &self.ticket,
            &self.dp.hosted,
            ComposeOptions {
                partial,
                note: Some("please test the export"),
            },
            60,
        )
    }
}

fn ticket_fixture(k: &str) -> crate::providers::RemoteTicket {
    ticket(k, "In Review")
}

#[test]
fn the_deploy_comment_lists_each_repo_with_pr_and_pipeline_and_needs_every_repo_deployed() {
    let an = Announce::new();
    // A second touched repo that is not pushed.
    links::add_manual(an.state(), &an.ticket, "worker", Some("b")).unwrap();
    prs::upsert(
        &an.dp.cache,
        &crate::providers::fake::build::pr("acme/api-client", 12, "feature/PROJ-50-x", "develop"),
        1,
    )
    .unwrap();
    an.dp.run(
        "run-1",
        &an.merge,
        PipelineState::Succeeded,
        Some(("alpha", PipelineState::Succeeded)),
        10,
    );

    // Refused while worker is not pushed, and the reason says which.
    let err = an.compose(false).unwrap_err().to_string();
    assert!(
        err.contains("worker: not pushed") && err.contains("--partial"),
        "{err}"
    );

    // Partial: allowed on request, and the body says what is pending.
    let draft = an.compose(true).unwrap();
    assert_eq!(draft.status, DraftStatus::Draft);
    assert!(
        draft
            .body
            .contains("PARTIALLY deployed. Pending: worker (not pushed)"),
        "{}",
        draft.body
    );
    assert!(draft.body.contains("api-client (acme/api-client)"));
    assert!(
        draft
            .body
            .contains("PR #12: https://bb.test/acme/api-client/pull-requests/12")
    );
    assert!(
        draft
            .body
            .contains("Pipeline #10: https://bb.test/p/run-1  (environment: alpha")
    );
    assert!(draft.body.contains("Not deployed yet (not pushed)"));
    assert!(draft.body.contains("please test the export"));

    // Nothing deployed at all: nothing to announce, even with --partial.
    let none = Announce::new();
    let err = none.compose(true).unwrap_err().to_string();
    assert!(err.contains("Nothing is deployed yet"), "{err}");
}

#[test]
fn a_fully_deployed_ticket_gets_a_full_comment_that_is_posted_verbatim_exactly_once() {
    let an = Announce::new();
    an.dp.run(
        "run-9",
        &an.later,
        PipelineState::Succeeded,
        Some(("alpha", PipelineState::Succeeded)),
        10,
    );
    let draft = an.compose(false).unwrap();
    assert!(
        draft.body.starts_with("PROJ-50 is deployed to alpha."),
        "{}",
        draft.body
    );
    assert!(!draft.body.contains("PARTIAL"));

    // The user edits it; the edit is what gets sent, byte for byte.
    let edited = "Deployed to alpha.\n\n  api-client: https://bb.test/p/run-9 \nThanks!\n";
    update_draft_body(an.state(), draft.id, edited).unwrap();
    assert!(update_draft_body(an.state(), draft.id, "  ").is_err());

    let gw = an.gateway();
    let preview = preview_post_comment(&gw, an.state(), draft.id).unwrap();
    assert_eq!(preview.preview().payload, edited);
    assert!(
        an.jira.log().writes().is_empty(),
        "previewing writes nothing"
    );

    // Editing after the preview invalidates the confirmation.
    let stale = preview.confirm();
    update_draft_body(an.state(), draft.id, "changed my mind").unwrap();
    let err = post_comment(&gw, an.state(), draft.id, stale, 70)
        .unwrap_err()
        .to_string();
    assert!(err.contains("preview it again"), "{err}");
    assert!(an.jira.log().writes().is_empty());

    update_draft_body(an.state(), draft.id, edited).unwrap();
    let confirmed = preview_post_comment(&gw, an.state(), draft.id)
        .unwrap()
        .confirm();
    let posted = post_comment(&gw, an.state(), draft.id, confirmed, 71).unwrap();

    assert_eq!(an.jira.log().writes().len(), 1);
    assert_eq!(
        an.jira.log().args_of("add_comment"),
        [format!("PROJ-50: {edited}")]
    );
    assert_eq!(posted.body_text, edited);
    let stored = drafts::get(an.state(), draft.id).unwrap().unwrap();
    assert_eq!(stored.status, DraftStatus::Posted);
    assert_eq!(stored.remote_id.as_deref(), Some(posted.id.as_str()));

    // A posted draft cannot be posted or edited again.
    assert!(
        preview_post_comment(&gw, an.state(), draft.id)
            .unwrap_err()
            .to_string()
            .contains("already posted")
    );
    assert!(update_draft_body(an.state(), draft.id, "again").is_err());
    assert_eq!(an.jira.log().writes().len(), 1);
}

#[test]
fn the_audit_log_alone_stops_a_double_post_even_if_the_draft_row_was_not_updated() {
    let an = Announce::new();
    an.dp.run(
        "run-9",
        &an.merge,
        PipelineState::Succeeded,
        Some(("alpha", PipelineState::Succeeded)),
        10,
    );
    let draft = an.compose(false).unwrap();
    let gw = an.gateway();
    let confirmed = preview_post_comment(&gw, an.state(), draft.id)
        .unwrap()
        .confirm();
    post_comment(&gw, an.state(), draft.id, confirmed, 71).unwrap();
    // Simulate the crash window: the comment went out but the row still says Draft.
    an.state()
        .conn()
        .execute(
            "UPDATE drafts SET status = 'draft', posted_at = NULL WHERE id = ?1",
            [draft.id],
        )
        .unwrap();
    let err = preview_post_comment(&gw, an.state(), draft.id)
        .unwrap_err()
        .to_string();
    assert!(err.contains("audit log shows"), "{err}");
    assert_eq!(an.jira.log().count("add_comment"), 1);
}

#[test]
fn the_transition_is_offered_only_after_the_comment_is_posted_and_uses_the_configured_status() {
    let an = Announce::new();
    an.dp.run(
        "run-9",
        &an.merge,
        PipelineState::Succeeded,
        Some(("alpha", PipelineState::Succeeded)),
        10,
    );
    let config = Config::parse("[jira.statuses]\nalpha_testing = \"Alpha QA\"\n").unwrap();
    let gw = an.gateway();

    // Before the comment: refused.
    let err = preview_transition(&gw, an.state(), &config, &an.ticket, 80)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Post the deploy comment"), "{err}");
    let draft = an.compose(false).unwrap();
    assert!(
        preview_transition(&gw, an.state(), &config, &an.ticket, 80).is_err(),
        "a draft is not posted"
    );

    let confirmed = preview_post_comment(&gw, an.state(), draft.id)
        .unwrap()
        .confirm();
    post_comment(&gw, an.state(), draft.id, confirmed, 81).unwrap();
    assert_eq!(an.jira.log().count("transition"), 0);

    let (id, preview) = preview_transition(&gw, an.state(), &config, &an.ticket, 82).unwrap();
    assert_eq!(preview.preview().payload, "PROJ-50 -> Alpha QA");
    // A comment confirmation cannot be used for it.
    let wrong = gw
        .draft(Action::ApprovePr {
            repo: "x/y".into(),
            pr: 1,
        })
        .confirm();
    assert!(post_transition(&gw, an.state(), id, wrong, 83).is_err());

    post_transition(&gw, an.state(), id, preview.confirm(), 84).unwrap();
    assert_eq!(an.jira.log().args_of("transition"), ["PROJ-50 -> Alpha QA"]);
    assert_eq!(an.jira.ticket(&an.ticket).unwrap().status, "Alpha QA");
    assert_eq!(
        drafts::get(an.state(), id).unwrap().unwrap().status,
        DraftStatus::Posted
    );

    // Once per posted comment; a new posted comment re-enables it.
    let err = preview_transition(&gw, an.state(), &config, &an.ticket, 85)
        .unwrap_err()
        .to_string();
    assert!(err.contains("already transitioned"), "{err}");
    let again = an.compose(false).unwrap();
    let c = preview_post_comment(&gw, an.state(), again.id)
        .unwrap()
        .confirm();
    post_comment(&gw, an.state(), again.id, c, 86).unwrap();
    assert!(preview_transition(&gw, an.state(), &config, &an.ticket, 87).is_ok());

    // Default status name without configuration.
    let default = preview_transition(&gw, an.state(), &Config::default(), &an.ticket, 88).unwrap();
    assert_eq!(default.1.preview().payload, "PROJ-50 -> Alpha Testing");
}

#[test]
fn an_unavailable_jira_adapter_fails_before_a_draft_is_offered() {
    let an = Announce::new();
    an.dp.run(
        "run-9",
        &an.merge,
        PipelineState::Succeeded,
        Some(("alpha", PipelineState::Succeeded)),
        10,
    );
    let draft = an.compose(false).unwrap();
    let missing = Gateway::with_writers(
        an.state(),
        Err(ProviderError::not_installed(
            "acli",
            "adapter not available in this build",
        )),
        Err(ProviderError::not_installed("bkt", "n/a")),
    );
    let err = preview_post_comment(&missing, an.state(), draft.id)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not available"), "{err}");
    assert_eq!(
        drafts::get(an.state(), draft.id).unwrap().unwrap().status,
        DraftStatus::Draft
    );
}

#[test]
fn recomposing_replaces_the_open_draft_and_discarded_drafts_cannot_be_posted() {
    let an = Announce::new();
    an.dp.run(
        "run-9",
        &an.merge,
        PipelineState::Succeeded,
        Some(("alpha", PipelineState::Succeeded)),
        10,
    );
    let first = an.compose(false).unwrap();
    let second = an.compose(false).unwrap();
    assert_eq!(
        drafts::get(an.state(), first.id).unwrap().unwrap().status,
        DraftStatus::Discarded
    );
    let gw = an.gateway();
    assert!(preview_post_comment(&gw, an.state(), first.id).is_err());
    discard_draft(an.state(), second.id).unwrap();
    assert!(preview_post_comment(&gw, an.state(), second.id).is_err());
    assert!(an.jira.log().writes().is_empty());
}

fn _unused(_: PathBuf) {}

//! The whole workflow, end to end, with real git repositories (bare origins), a real
//! `state.db`, the real sync engine and integration flow, and the provider fakes. At every
//! step the top suggestion is asserted, and so is the absence of steps already done.

use std::cell::Cell;

use tempfile::TempDir;

use super::{
    exec::{ExternalOutcome, LocalEnv, LocalOutcome, execute_local, prepare_external, respond},
    *,
};
use crate::{
    activation::WorkspaceRepo,
    config::Config,
    domain::{LocalStatus, TicketKey},
    gateway::{Gateway, PushReport},
    integration::{
        execute_push, finalize_integration, prepare_integration,
    },
    providers::{
        CodeHost, PipelineState, RemoteComment, Reviewer, TicketProvider,
        fake::{
            FakeBitbucket, FakeJira,
            build::{pr, run, ticket},
        },
    },
    store::{
        Kind, Store, drafts::{self, DraftKind, DraftStatus}, notes, prs, suggestion_responses,
        tickets, uat_merges,
    },
    sync::{DEFAULT_MIN_INTERVAL, HostedRepo, SyncContext, sync_all, uat_commits},
    testsupport::{Fixture, key},
};

struct World {
    fx: Fixture,
    cache: Store,
    data: TempDir,
    config: Config,
    hosted: Vec<HostedRepo>,
    jira: FakeJira,
    bb: FakeBitbucket,
    now: Cell<i64>,
    ticket: TicketKey,
}

impl World {
    fn new() -> Self {
        let fx = Fixture::new();
        fx.api_client
            .add_branch("feature/PROJ-1-api-change", "src/Client.php", false);
        fx.worker.add_branch("feature/PROJ-1-work", "w.txt", false);
        fx.api_client.add_uat();
        fx.worker.add_uat();

        let hosted = [(&fx.api_client, "acme/api-client"), (&fx.worker, "acme/worker")]
            .into_iter()
            .map(|(r, host)| HostedRepo {
                project: r.name.clone(),
                repo: host.into(),
                dir: r.dir.clone(),
                branches: Default::default(),
                deploy_environment: Some("alpha".into()),
            })
            .collect();

        let jira = FakeJira::new();
        jira.on_search("REVIEW", vec![ticket("PROJ-1", "In Review")]);
        let bb = FakeBitbucket::new();
        bb.add_pr(pr("acme/api-client", 1, "feature/PROJ-1-api-change", "develop"));
        bb.add_pr(pr("acme/worker", 2, "feature/PROJ-1-work", "develop"));

        Self {
            fx,
            cache: Store::open_in_memory(Kind::Cache).unwrap(),
            data: TempDir::new().unwrap(),
            config: Config::parse(
                "[jira]\nreview_jql = \"REVIEW\"\naccount_id = \"me\"\n[bitbucket]\nworkspace = \"acme\"\n",
            )
            .unwrap(),
            hosted,
            jira,
            bb,
            now: Cell::new(1_000),
            ticket: key("PROJ-1"),
        }
    }

    /// Time moves on a little at every use.
    fn tick(&self) -> i64 {
        self.now.set(self.now.get() + 10);
        self.now.get()
    }

    fn repos(&self) -> Vec<WorkspaceRepo> {
        self.fx.repos()
    }

    fn gateway(&self) -> Gateway<'_> {
        Gateway::with_writers(
            &self.fx.store,
            Ok(Box::new(self.jira.clone())),
            Ok(Box::new(self.bb.clone())),
        )
    }

    fn sync(&self) {
        let ctx = SyncContext {
            state: &self.fx.store,
            cache: &self.cache,
            config: &self.config,
            now: self.tick(),
            force: true,
            min_interval: DEFAULT_MIN_INTERVAL,
        };
        let commits = uat_commits(&self.fx.store, &self.hosted).unwrap();
        let report = sync_all(
            &ctx,
            Ok(&self.jira as &dyn TicketProvider),
            Ok(&self.bb as &dyn CodeHost),
            &self.hosted,
            &commits,
            None,
        );
        assert!(report.is_ok(), "{report:?}");
    }

    fn snapshot(&self) -> Snapshot {
        let repos = self.repos();
        let gateway = self.gateway();
        Snapshot::load(&LoadContext {
            state: &self.fx.store,
            cache: &self.cache,
            config: &self.config,
            repos: &repos,
            hosted: &self.hosted,
            gateway: &gateway,
        })
        .unwrap()
    }

    /// The ranked list right now. Also proves that ranking writes nothing anywhere.
    fn list(&self) -> Vec<Suggestion> {
        let writes = (self.jira.log().writes().len(), self.bb.log().writes().len());
        let audit_before = crate::store::audit::list(&self.fx.store, None, 10_000)
            .unwrap()
            .len();
        let snapshot = self.snapshot();
        let now = self.now.get();
        let first = suggest(&snapshot, now);
        assert_eq!(first, suggest(&snapshot, now), "suggest is a pure function");
        assert_eq!(
            writes,
            (self.jira.log().writes().len(), self.bb.log().writes().len()),
            "no writer method was called"
        );
        assert_eq!(
            audit_before,
            crate::store::audit::list(&self.fx.store, None, 10_000)
                .unwrap()
                .len(),
            "loading and ranking append no audit entry"
        );
        first
    }

    fn assert_top(&self, rule: RuleId) -> Suggestion {
        let list = self.list();
        let got: Vec<_> = list.iter().map(|s| (s.rule, s.id.clone())).collect();
        assert_eq!(
            list.first().map(|s| s.rule),
            Some(rule),
            "expected {rule} on top, got {got:#?}"
        );
        list.into_iter().next().unwrap()
    }

    fn assert_absent(&self, rule: RuleId) {
        assert!(
            !self.list().iter().any(|s| s.rule == rule),
            "{rule} should not be suggested any more"
        );
    }

    fn env<'a>(&'a self, repos: &'a [WorkspaceRepo]) -> LocalEnv<'a> {
        LocalEnv {
            state: &self.fx.store,
            cache: &self.cache,
            repos,
            hosted: &self.hosted,
            runner: &self.fx.runner,
            workspace_default_branch: None,
            fetch: false,
        }
    }

    fn run_local(&self, s: &Suggestion) -> LocalOutcome {
        let repos = self.repos();
        execute_local(&self.env(&repos), &s.action, self.tick()).unwrap()
    }

    fn run_external(&self, s: &Suggestion) -> ExternalOutcome {
        let gateway = self.gateway();
        let prepared =
            prepare_external(&gateway, &self.fx.store, &self.config, s, self.tick()).unwrap();
        assert!(!prepared.preview().payload.is_empty());
        // The human says yes here.
        prepared
            .confirm()
            .execute(&gateway, &self.fx.store, self.tick())
            .unwrap()
    }

    fn status(&self) -> LocalStatus {
        tickets::get(&self.fx.store, &self.ticket)
            .unwrap()
            .unwrap()
            .status
    }

    fn merge_commit(&self, project: &str) -> String {
        uat_merges::list_for_ticket(&self.fx.store, &self.ticket)
            .unwrap()
            .into_iter()
            .rfind(|m| m.repo == project)
            .unwrap()
            .commit
    }

    fn pipeline(&self, project: &str, id: &str, state: PipelineState) {
        let host = format!("acme/{project}");
        let mut r = run(&host, id, "uat", &self.merge_commit(project), self.now.get());
        r.state = state.clone();
        r.number = Some(3);
        r.steps[0].state = state;
        self.bb.add_pipeline(r);
    }
}

#[test]
fn the_top_suggestion_follows_the_workflow_from_review_to_uat_sign_off() {
    let w = World::new();

    // 0. Nothing has ever been synced.
    assert!(w.snapshot().tickets.is_empty());
    let sync = w.assert_top(RuleId::SyncStale);
    assert_eq!(sync.level, ExecutionLevel::Automatic);

    // 1. The ticket is in Review: claim it.
    w.sync();
    let claim = w.assert_top(RuleId::ClaimNew);
    assert_eq!(claim.action, SuggestedAction::Claim { ticket: w.ticket.clone() });
    w.run_local(&claim);
    assert_eq!(w.status(), LocalStatus::Claimed);
    w.assert_absent(RuleId::ClaimNew);

    // 2. Claimed: start the review, then finish it.
    let start = w.assert_top(RuleId::StartReview);
    assert_eq!(start.facts["prs"].as_array().unwrap().len(), 2);
    w.run_local(&start);
    assert_eq!(w.status(), LocalStatus::Reviewing);
    w.assert_absent(RuleId::StartReview);

    let finish = w.assert_top(RuleId::FinishReview);
    w.run_local(&finish);
    w.assert_absent(RuleId::FinishReview);
    let mark = crate::store::reviews::get(&w.fx.store, &w.ticket).unwrap().unwrap();
    assert_eq!(mark.heads.len(), 2, "both branch tips were recorded");
    w.assert_absent(RuleId::ReReview);

    // 3. Reviewed and nothing active: activate.
    let activate = w.assert_top(RuleId::ActivateReviewed);
    assert_eq!(
        activate.action,
        SuggestedAction::Activate { ticket: w.ticket.clone(), baseline: None }
    );
    assert!(matches!(w.run_local(&activate), LocalOutcome::Activated(_)));
    assert_eq!(w.status(), LocalStatus::Active);
    w.assert_absent(RuleId::ActivateReviewed);
    assert!(
        w.list().is_empty(),
        "an active ticket being tested has no next step: {:#?}",
        w.list()
    );

    // 4. The checklist fills up; only a complete one suggests integrating.
    let a = notes::add_checklist_item(&w.fx.store, &w.ticket, "login works").unwrap();
    let b = notes::add_checklist_item(&w.fx.store, &w.ticket, "export works").unwrap();
    notes::set_checklist_done(&w.fx.store, a.id, true).unwrap();
    w.assert_absent(RuleId::IntegrateReady);
    notes::set_checklist_done(&w.fx.store, b.id, true).unwrap();
    let integrate = w.assert_top(RuleId::IntegrateReady);
    assert_eq!(
        integrate.action,
        SuggestedAction::RunIntegrationPrepare { ticket: w.ticket.clone() }
    );

    // The preparation, the confirmed push and the finalize are the integration flow's own.
    let repos = w.repos();
    let prep = prepare_integration(
        &w.fx.store,
        &w.fx.runner,
        w.data.path(),
        &w.ticket,
        &repos,
        w.tick(),
    )
    .unwrap();
    assert!(prep.is_ready());
    w.assert_top(RuleId::IntegrateReady);
    let gateway = w.gateway();
    let draft = gateway.draft(prep.push_action().unwrap());
    let (report, _): (PushReport, _) = execute_push(&gateway, draft.confirm(), w.tick()).unwrap();
    assert!(report.all_done());
    finalize_integration(&w.fx.store, &w.fx.runner, w.data.path(), &prep, &repos, w.tick())
        .unwrap();
    assert_eq!(w.status(), LocalStatus::Integrated);
    w.assert_absent(RuleId::IntegrateReady);
    w.assert_absent(RuleId::RevertOverlay);

    // 5. Pipelines: nothing known yet, then running, then failed, then deployed.
    w.sync();
    let waiting = w.assert_top(RuleId::DeployWaiting);
    assert!(waiting.informational);
    w.pipeline("api-client", "{a1}", PipelineState::Running);
    w.pipeline("worker", "{w1}", PipelineState::Running);
    w.sync();
    let waiting = w.assert_top(RuleId::DeployWaiting);
    assert!(waiting.reason.contains("running"));

    w.pipeline("api-client", "{a1}", PipelineState::Failed);
    w.sync();
    let failed = w.assert_top(RuleId::DeployFailed);
    assert!(matches!(failed.action, SuggestedAction::OpenPipeline { .. }));
    w.assert_absent(RuleId::DeployWaiting);
    let rerun = w
        .list()
        .into_iter()
        .find(|s| matches!(s.action, SuggestedAction::Gateway(_)))
        .unwrap();
    assert_eq!(rerun.level, ExecutionLevel::ConfirmedExternal);
    assert!(matches!(w.run_external(&rerun), ExternalOutcome::Executed(_)));
    assert_eq!(w.bb.log().count("rerun_pipeline"), 1);

    w.pipeline("api-client", "{a1}", PipelineState::Succeeded);
    w.pipeline("worker", "{w1}", PipelineState::Succeeded);
    w.sync();
    w.assert_absent(RuleId::DeployFailed);
    let compose = w.assert_top(RuleId::ComposeDeployComment);
    assert_eq!(
        compose.action,
        SuggestedAction::ComposeDeployComment { ticket: w.ticket.clone(), partial: false }
    );

    // 6. The draft is composed, then posted, then the transition.
    assert!(matches!(w.run_local(&compose), LocalOutcome::Drafted(_)));
    w.assert_absent(RuleId::ComposeDeployComment);
    let post = w.assert_top(RuleId::PostDeployComment);
    assert_eq!(post.level, ExecutionLevel::ConfirmedExternal);
    assert_eq!(w.jira.log().writes().len(), 0, "drafting wrote nothing");
    assert!(matches!(w.run_external(&post), ExternalOutcome::CommentPosted(_)));
    assert_eq!(w.jira.log().count("add_comment"), 1);
    assert!(
        drafts::latest(&w.fx.store, &w.ticket, DraftKind::DeployComment, DraftStatus::Posted)
            .unwrap()
            .is_some()
    );
    w.assert_absent(RuleId::PostDeployComment);
    w.assert_absent(RuleId::ComposeDeployComment);

    let transition = w.assert_top(RuleId::TransitionAlpha);
    assert!(matches!(w.run_external(&transition), ExternalOutcome::Transitioned));
    assert_eq!(w.jira.ticket(&w.ticket).unwrap().status, "Alpha Testing");
    w.assert_absent(RuleId::TransitionAlpha);
    w.sync();
    assert!(w.list().is_empty(), "everything is done: {:#?}", w.list());
    assert_eq!(w.jira.log().writes().len(), 2);

    // 7. A fix lands on the ticket branch after alpha: merge again.
    let api = &w.fx.api_client;
    api.git(&["switch", "feature/PROJ-1-api-change"]);
    api.commit("src/Fix.php", "fix\n", "fix after alpha");
    api.git(&["push", "origin", "feature/PROJ-1-api-change"]);
    api.git(&["switch", "develop"]);
    let again = w.assert_top(RuleId::RemergeNeeded);
    assert_eq!(
        again.action,
        SuggestedAction::Activate { ticket: w.ticket.clone(), baseline: None }
    );
    assert!(again.reason.contains("api-client"));

    // 8. The ticket is Returned and the author is mentioned: that wins.
    let mut returned = ticket("PROJ-1", "Returned");
    returned.updated_at = w.now.get();
    w.jira.add_ticket(returned.clone());
    w.jira.on_search("REVIEW", vec![returned]);
    let mention = RemoteComment {
        id: "c1".into(),
        ticket: w.ticket.clone(),
        author_account_id: "sam".into(),
        author_name: "Sam".into(),
        body_text: "@me this still breaks export".into(),
        mentions: vec!["me".into()],
        created_at: w.now.get() + 5,
    };
    w.jira.set_comments(&w.ticket, vec![mention.clone()]);
    w.sync();
    let surfaced = w.assert_top(RuleId::ReturnedMention);
    assert_eq!(surfaced.facts["comment_id"], "c1");
    assert!(surfaced.reason.contains("still breaks export"));

    // Dismissed, it stays away until a newer mention arrives.
    respond(
        &w.fx.store,
        &surfaced,
        suggestion_responses::ResponseKind::Dismissed,
        Some("known".into()),
        None,
        w.tick(),
    )
    .unwrap();
    let responses = suggestion_responses::latest_map(&w.fx.store).unwrap();
    let visible = next_actions(&w.snapshot(), &responses, w.now.get());
    assert_eq!(visible[0].rule, RuleId::RemergeNeeded);
    let mut second = mention;
    second.id = "c2".into();
    second.created_at = w.now.get() + 5;
    w.jira.set_comments(&w.ticket, vec![second]);
    w.sync();
    let visible = next_actions(&w.snapshot(), &responses, w.now.get());
    assert_eq!(visible[0].rule, RuleId::ReturnedMention, "new facts resurface it");

    // 9. Back through UAT: signed off, so the PRs can be approved.
    let mut signed_off = ticket("PROJ-1", "UAT");
    signed_off.updated_at = w.now.get();
    w.jira.add_ticket(signed_off.clone());
    w.jira.on_search("REVIEW", vec![signed_off]);
    w.sync();
    w.assert_absent(RuleId::ReturnedMention);
    let approve = w.assert_top(RuleId::ApprovePrs);
    assert_eq!(approve.facts["repo"], "acme/api-client");
    assert!(matches!(w.run_external(&approve), ExternalOutcome::Executed(_)));
    assert_eq!(w.bb.log().args_of("approve"), ["acme/api-client #1"]);

    // The host now shows my approval; only the other PR is left.
    let mut done = pr("acme/api-client", 1, "feature/PROJ-1-api-change", "develop");
    done.reviewers = vec![Reviewer {
        account: "me".into(),
        approved: true,
        changes_requested: false,
    }];
    w.bb.add_pr(done);
    w.sync();
    let left: Vec<_> = w
        .list()
        .into_iter()
        .filter(|s| s.rule == RuleId::ApprovePrs)
        .collect();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].facts["repo"], "acme/worker");
    assert_eq!(prs::for_ticket(&w.cache, &w.ticket).unwrap().len(), 2);

    // Exactly the confirmed writes happened: a comment and a transition in Jira, a pipeline
    // re-run and an approval on the code host.
    assert_eq!(w.jira.log().writes().len(), 2);
    assert_eq!(w.bb.log().writes().len(), 2);
}

#[test]
fn an_unavailable_writer_shows_up_as_a_note_when_loading() {
    let w = World::new();
    w.sync();
    let missing = || crate::providers::ProviderError::not_installed("acli", "not installed");
    let gateway = Gateway::with_writers(&w.fx.store, Err(missing()), Ok(Box::new(w.bb.clone())));
    let repos = w.repos();
    let snapshot = Snapshot::load(&LoadContext {
        state: &w.fx.store,
        cache: &w.cache,
        config: &w.config,
        repos: &repos,
        hosted: &w.hosted,
        gateway: &gateway,
    })
    .unwrap();
    assert!(matches!(snapshot.writers.jira, Availability::Unavailable(_)));
    assert!(snapshot.writers.code_host.is_ready());
}

#[test]
fn a_prepared_conflict_is_shown_until_the_branch_moves() {
    let w = World::new();
    w.sync();
    // Two tickets touching the same file: uat gets PROJ-9's version first.
    let api = &w.fx.api_client;
    api.git(&["switch", "-c", "feature/PROJ-9-other", "develop"]);
    api.commit("src/Client.php", "other\n", "other ticket");
    api.git(&["push", "origin", "feature/PROJ-9-other:uat"]);
    api.git(&["switch", "develop"]);

    let claim = w.assert_top(RuleId::ClaimNew);
    w.run_local(&claim);
    let start = w.assert_top(RuleId::StartReview);
    w.run_local(&start);
    let finish = w.assert_top(RuleId::FinishReview);
    w.run_local(&finish);
    let activate = w.assert_top(RuleId::ActivateReviewed);
    w.run_local(&activate);
    let item = notes::add_checklist_item(&w.fx.store, &w.ticket, "works").unwrap();
    notes::set_checklist_done(&w.fx.store, item.id, true).unwrap();

    let repos = w.repos();
    let prep = prepare_integration(
        &w.fx.store,
        &w.fx.runner,
        w.data.path(),
        &w.ticket,
        &repos,
        w.tick(),
    )
    .unwrap();
    assert!(!prep.is_pushable(), "{prep:#?}");

    let list = w.list();
    assert_eq!(list[0].rule, RuleId::IntegrationBlocked);
    assert!(matches!(list[0].action, SuggestedAction::ResolveConflict { .. }));
    assert_eq!(list[1].rule, RuleId::IntegrateReady, "retry stays available");

    // The ticket branch moves on: the old result no longer describes it.
    api.git(&["switch", "feature/PROJ-1-api-change"]);
    api.commit("src/Extra.php", "x\n", "more work");
    api.git(&["switch", "develop"]);
    w.assert_absent(RuleId::IntegrationBlocked);
}

//! Unit tests of every rule against hand-built snapshots, plus ordering, ids, the response
//! filter and the "adapter not available" replacement.

use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::{
    domain::{LocalStatus, TicketKey, TicketKind},
    gateway::Action,
    integration::DeployState,
    providers::{
        Pr, PrState, Reviewer,
        fake::build::{key, pr},
    },
    store::suggestion_responses::{ResponseKind, SuggestionResponse},
};

const NOW: i64 = 1_000_000;

fn snap(tickets: Vec<TicketSnapshot>) -> Snapshot {
    Snapshot {
        me: Some("me".into()),
        tickets,
        ..Snapshot::default()
    }
}

fn pool(k: &str, priority: Option<&str>) -> TicketSnapshot {
    let mut t = TicketSnapshot::new(key(k));
    t.jira_status = Some("In Review".into());
    t.jira_priority = priority.map(String::from);
    t
}

fn mine(k: &str, status: LocalStatus, order: i64) -> TicketSnapshot {
    let mut t = TicketSnapshot::new(key(k));
    t.tracking = Some(Tracking {
        status,
        manual_order: order,
        claimed_at: 1,
        updated_at: NOW - 100,
    });
    t.jira_status = Some("In Review".into());
    t
}

fn open_pr(project: &str, repo: &str, id: u64, dest: &str) -> PrInfo {
    PrInfo {
        pr: pr(repo, id, "feature/X-1", dest),
        project: Some(project.into()),
    }
}

fn rules_of(s: &[Suggestion]) -> Vec<RuleId> {
    s.iter().map(|x| x.rule).collect()
}

fn only(s: &Snapshot, rule: RuleId) -> Vec<Suggestion> {
    suggest(s, NOW)
        .into_iter()
        .filter(|x| x.rule == rule)
        .collect()
}

fn deploy(repo: &str, state: DeployState) -> DeployInfo {
    DeployInfo {
        repo: repo.into(),
        hosting_repo: Some(format!("acme/{repo}")),
        state,
        run_id: Some(format!("{{{repo}-run}}")),
        run_number: Some(7),
        run_url: Some(format!("https://bb/{repo}/7")),
    }
}

fn merged(t: &mut TicketSnapshot, repo: &str, at: i64) {
    t.merges.push(MergeInfo {
        repo: repo.into(),
        commit: "c".repeat(40),
        recorded_at: at,
    });
}

// ----------------------------------------------------------------- sync

#[test]
fn sync_is_suggested_for_missing_or_old_sources_only() {
    let mut s = snap(vec![]);
    s.sync.expected = vec!["jira".into(), "bitbucket:acme/web".into()];
    let list = only(&s, RuleId::SyncStale);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].action, SuggestedAction::Sync);
    assert_eq!(list[0].level, ExecutionLevel::Automatic);

    let fresh = |src: &str, ok: i64| SourceState {
        source: src.into(),
        last_ok_at: Some(ok),
        last_attempt_at: ok,
        last_error: None,
    };
    s.sync.states = vec![
        fresh("jira", NOW - 10),
        fresh("bitbucket:acme/web", NOW - 20),
    ];
    assert!(only(&s, RuleId::SyncStale).is_empty());

    // Old data: stale.
    s.sync.states[0] = fresh("jira", NOW - 301);
    assert_eq!(only(&s, RuleId::SyncStale).len(), 1);

    // A recent failed attempt is not retried at once.
    s.sync.states[0] = SourceState {
        source: "jira".into(),
        last_ok_at: None,
        last_attempt_at: NOW - 30,
        last_error: Some("offline".into()),
    };
    assert!(only(&s, RuleId::SyncStale).is_empty());
    // A failing source backs off for 15 minutes, not one.
    s.sync.states[0].last_attempt_at = NOW - 61;
    assert!(only(&s, RuleId::SyncStale).is_empty());
    s.sync.states[0].last_attempt_at = NOW - 899;
    assert!(only(&s, RuleId::SyncStale).is_empty());
    s.sync.states[0].last_attempt_at = NOW - 901;
    assert_eq!(only(&s, RuleId::SyncStale).len(), 1);
}

#[test]
fn a_persistently_partial_source_backs_off_but_a_merely_stale_one_does_not() {
    let mut s = snap(vec![]);
    s.sync.expected = vec!["jira".into()];
    // Partial: it has synced before (old data) but the last attempt ended in an error.
    let partial = |attempt_age: i64| SourceState {
        source: "jira".into(),
        last_ok_at: Some(NOW - 5000),
        last_attempt_at: NOW - attempt_age,
        last_error: Some("2 tickets failed".into()),
    };
    s.sync.states = vec![partial(120)];
    assert!(only(&s, RuleId::SyncStale).is_empty());
    s.sync.states = vec![partial(16 * 60)];
    assert_eq!(only(&s, RuleId::SyncStale).len(), 1);

    // Stale but the last attempt succeeded (no error): back to the one-minute retry.
    s.sync.states = vec![SourceState {
        source: "jira".into(),
        last_ok_at: Some(NOW - 5000),
        last_attempt_at: NOW - 120,
        last_error: None,
    }];
    assert_eq!(only(&s, RuleId::SyncStale).len(), 1);
}

#[test]
fn nothing_expected_means_no_sync_suggestion() {
    assert!(suggest(&snap(vec![]), NOW).is_empty());
}

// ---------------------------------------------------------------- claims

#[test]
fn review_tickets_are_claimed_by_jira_priority_then_key() {
    let s = snap(vec![
        pool("PROJ-9", Some("Low")),
        pool("PROJ-3", Some("High")),
        pool("PROJ-2", Some("High")),
        pool("PROJ-7", None),
    ]);
    let list = only(&s, RuleId::ClaimNew);
    let keys: Vec<&str> = list
        .iter()
        .map(|x| x.ticket.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(keys, ["PROJ-2", "PROJ-3", "PROJ-9", "PROJ-7"]);
    assert_eq!(list[0].priority, Priority(470));
    assert_eq!(list[2].priority, Priority(450));
    assert!(list[0].reason.contains("#1 in the queue"));
    assert!(
        list.iter()
            .all(|x| x.level == ExecutionLevel::LocalOneClick)
    );
}

#[test]
fn claiming_skips_claimed_and_non_review_tickets() {
    let mut other = pool("PROJ-2", None);
    other.jira_status = Some("Alpha Testing".into());
    let s = snap(vec![
        mine("PROJ-1", LocalStatus::Claimed, 0),
        other,
        pool("PROJ-3", None),
    ]);
    let list = only(&s, RuleId::ClaimNew);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].ticket, Some(key("PROJ-3")));
}

#[test]
fn review_status_matches_case_insensitively_and_by_config() {
    let mut t = pool("PROJ-1", None);
    t.jira_status = Some(" in review ".into());
    assert_eq!(only(&snap(vec![t.clone()]), RuleId::ClaimNew).len(), 1);

    let mut s = snap(vec![t]);
    s.statuses.review = "Code Review".into();
    assert!(only(&s, RuleId::ClaimNew).is_empty());
}

#[test]
fn a_returned_ticket_back_in_review_is_claimed_again_only_after_the_transition() {
    let mut t = mine("PROJ-1", LocalStatus::Integrated, 0);
    t.transition_posted_at = Some(500);
    t.jira_fetched_at = Some(900);
    t.jira_status = Some("In Review".into());
    let list = only(&snap(vec![t.clone()]), RuleId::ReturnedToReview);
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].action,
        SuggestedAction::Claim {
            ticket: key("PROJ-1")
        }
    );

    // Integrated but not announced yet: Jira still says Review, that is normal.
    let mut early = t.clone();
    early.transition_posted_at = None;
    assert!(only(&snap(vec![early]), RuleId::ReturnedToReview).is_empty());

    // Jira row older than the transition: it may not have caught up.
    let mut stale = t.clone();
    stale.jira_fetched_at = Some(400);
    assert!(only(&snap(vec![stale]), RuleId::ReturnedToReview).is_empty());

    // Still in Alpha Testing.
    let mut alpha = t;
    alpha.jira_status = Some("Alpha Testing".into());
    assert!(only(&snap(vec![alpha]), RuleId::ReturnedToReview).is_empty());
}

// ---------------------------------------------------------------- review

#[test]
fn claimed_tickets_start_review_and_say_where_the_prs_go() {
    let mut t = mine("PROJ-1", LocalStatus::Claimed, 0);
    t.prs = vec![open_pr("web", "acme/web", 7, "develop")];
    let list = only(&snap(vec![t]), RuleId::StartReview);
    assert_eq!(list.len(), 1);
    assert!(list[0].reason.contains("acme/web #7 into develop"));

    let s = snap(vec![mine("PROJ-2", LocalStatus::Reviewing, 0)]);
    assert!(only(&s, RuleId::StartReview).is_empty());
    let s = snap(vec![mine("PROJ-2", LocalStatus::Claimed, 0)]);
    assert!(
        only(&s, RuleId::StartReview)[0]
            .reason
            .contains("no open PR")
    );
}

#[test]
fn finishing_a_review_is_suggested_until_it_is_marked() {
    let mut t = mine("PROJ-1", LocalStatus::Reviewing, 0);
    assert_eq!(only(&snap(vec![t.clone()]), RuleId::FinishReview).len(), 1);
    t.review.reviewed_at = Some(10);
    assert!(only(&snap(vec![t]), RuleId::FinishReview).is_empty());
}

#[test]
fn re_review_compares_branch_tips_before_pr_timestamps() {
    let mut t = mine("PROJ-1", LocalStatus::Reviewing, 0);
    let mut p = open_pr("web", "acme/web", 7, "develop");
    p.pr.updated_at = 900;
    t.prs = vec![p];
    t.review.reviewed_at = Some(500);
    t.review.reviewed_heads = BTreeMap::from([("web".to_string(), "a".repeat(40))]);
    t.review.current_heads = BTreeMap::from([("web".to_string(), "a".repeat(40))]);
    // Same tip: a newer PR timestamp (a comment) does not count.
    assert!(only(&snap(vec![t.clone()]), RuleId::ReReview).is_empty());

    t.review.current_heads.insert("web".into(), "b".repeat(40));
    let list = only(&snap(vec![t.clone()]), RuleId::ReReview);
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].action,
        SuggestedAction::StartReview {
            ticket: key("PROJ-1")
        }
    );
    assert!(list[0].reason.contains("aaaaaaa..bbbbbbb"));
}

#[test]
fn re_review_falls_back_to_pr_updated_at() {
    let mut t = mine("PROJ-1", LocalStatus::Reviewing, 0);
    let mut p = open_pr("web", "acme/web", 7, "develop");
    p.pr.updated_at = 400;
    t.prs = vec![p.clone()];
    t.review.reviewed_at = Some(500);
    assert!(only(&snap(vec![t.clone()]), RuleId::ReReview).is_empty());
    p.pr.updated_at = 501;
    t.prs = vec![p];
    assert_eq!(only(&snap(vec![t.clone()]), RuleId::ReReview).len(), 1);

    // Not reviewed at all: nothing to redo.
    t.review.reviewed_at = None;
    assert!(only(&snap(vec![t]), RuleId::ReReview).is_empty());
}

#[test]
fn re_review_ignores_tickets_past_testing() {
    let mut t = mine("PROJ-1", LocalStatus::Integrated, 0);
    let mut p = open_pr("web", "acme/web", 7, "develop");
    p.pr.updated_at = 900;
    t.prs = vec![p];
    t.review.reviewed_at = Some(500);
    assert!(only(&snap(vec![t]), RuleId::ReReview).is_empty());
}

// ------------------------------------------------------------- activation

fn reviewed(k: &str, status: LocalStatus, order: i64) -> TicketSnapshot {
    let mut t = mine(k, status, order);
    t.review.reviewed_at = Some(10);
    t
}

#[test]
fn a_reviewed_ticket_is_activated_when_nothing_is_active() {
    let s = snap(vec![reviewed("PROJ-1", LocalStatus::Reviewing, 0)]);
    let list = only(&s, RuleId::ActivateReviewed);
    assert_eq!(
        list[0].action,
        SuggestedAction::Activate {
            ticket: key("PROJ-1"),
            baseline: None
        }
    );

    // Reviewing but not reviewed: no.
    let s = snap(vec![mine("PROJ-1", LocalStatus::Reviewing, 0)]);
    assert!(only(&s, RuleId::ActivateReviewed).is_empty());
    // A parked ticket is resumed, at a lower priority.
    let s = snap(vec![mine("PROJ-1", LocalStatus::Parked, 0)]);
    assert_eq!(
        only(&s, RuleId::ActivateReviewed)[0].priority,
        Priority(630)
    );
}

#[test]
fn nothing_is_activated_while_another_ticket_is_active() {
    let s = snap(vec![
        mine("PROJ-1", LocalStatus::Active, 0),
        reviewed("PROJ-2", LocalStatus::Reviewing, 1),
    ]);
    let all = suggest(&s, NOW);
    assert!(
        !all.iter()
            .any(|x| matches!(x.action, SuggestedAction::Activate { .. }))
    );
    let park = only(&s, RuleId::ParkActive);
    assert_eq!(park.len(), 1);
    assert_eq!(
        park[0].action,
        SuggestedAction::Park {
            ticket: key("PROJ-1")
        }
    );
    assert!(park[0].reason.contains("PROJ-2"));
}

#[test]
fn a_hotfix_asks_for_its_baseline_first() {
    let mut t = reviewed("PROJ-1", LocalStatus::Reviewing, 0);
    t.kind = TicketKind::Hotfix;
    let list = only(&snap(vec![t]), RuleId::ActivateReviewed);
    assert_eq!(
        list[0].action,
        SuggestedAction::ChooseHotfixBaseline {
            ticket: key("PROJ-1")
        }
    );
    assert_eq!(list[0].facts["hotfix"], true);
    assert!(list[0].reason.starts_with("Hotfix: "));
    assert_eq!(list[0].priority, Priority(840));
}

#[test]
fn a_waiting_hotfix_raises_the_park_suggestion_and_dirty_trees_are_named() {
    let mut active = mine("PROJ-1", LocalStatus::Active, 0);
    active.dirty_repos = vec!["web".into()];
    let mut hot = reviewed("PROJ-2", LocalStatus::Reviewing, 1);
    hot.kind = TicketKind::Hotfix;
    let plain = snap(vec![
        active.clone(),
        reviewed("PROJ-2", LocalStatus::Reviewing, 1),
    ]);
    let calm = only(&plain, RuleId::ParkActive);
    assert_eq!(calm[0].priority, Priority(625));
    assert!(calm[0].reason.contains("stashed"));

    let urgent = only(&snap(vec![active.clone(), hot]), RuleId::ParkActive);
    assert_eq!(urgent[0].priority, Priority(825));

    // Nobody waiting: nothing to leave the active ticket for.
    assert!(only(&snap(vec![active]), RuleId::ParkActive).is_empty());
}

#[test]
fn stale_tickets_are_parked_or_finished() {
    let mut a = mine("PROJ-1", LocalStatus::Active, 0);
    a.tracking.as_mut().unwrap().updated_at = NOW - 6 * 86_400;
    let mut p = mine("PROJ-2", LocalStatus::Parked, 1);
    p.tracking.as_mut().unwrap().updated_at = NOW - 15 * 86_400;
    let list = only(&snap(vec![a]), RuleId::StaleTicket);
    assert_eq!(
        list[0].action,
        SuggestedAction::Park {
            ticket: key("PROJ-1")
        }
    );
    let list = only(&snap(vec![p]), RuleId::StaleTicket);
    assert_eq!(
        list[0].action,
        SuggestedAction::MarkDone {
            ticket: key("PROJ-2")
        }
    );

    let mut fresh = mine("PROJ-1", LocalStatus::Active, 0);
    fresh.tracking.as_mut().unwrap().updated_at = NOW - 4 * 86_400;
    assert!(only(&snap(vec![fresh]), RuleId::StaleTicket).is_empty());
    let mut parked = mine("PROJ-2", LocalStatus::Parked, 1);
    parked.tracking.as_mut().unwrap().updated_at = NOW - 13 * 86_400;
    assert!(only(&snap(vec![parked]), RuleId::StaleTicket).is_empty());
}

// ------------------------------------------------------------ integration

fn active_done() -> TicketSnapshot {
    let mut t = mine("PROJ-1", LocalStatus::Active, 0);
    t.checklist = Checklist { total: 2, done: 2 };
    t.touched_repos = vec!["web".into()];
    t
}

#[test]
fn a_complete_checklist_suggests_preparing_the_integration_not_the_push() {
    let list = only(&snap(vec![active_done()]), RuleId::IntegrateReady);
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].action,
        SuggestedAction::RunIntegrationPrepare {
            ticket: key("PROJ-1")
        }
    );
    assert_eq!(list[0].level, ExecutionLevel::LocalOneClick);
}

#[test]
fn integration_needs_a_complete_checklist_touched_repos_and_an_active_ticket() {
    let mut partial = active_done();
    partial.checklist.done = 1;
    assert!(only(&snap(vec![partial]), RuleId::IntegrateReady).is_empty());
    let mut empty = active_done();
    empty.checklist = Checklist::default();
    assert!(only(&snap(vec![empty]), RuleId::IntegrateReady).is_empty());
    let mut untouched = active_done();
    untouched.touched_repos.clear();
    assert!(only(&snap(vec![untouched]), RuleId::IntegrateReady).is_empty());
    let mut parked = active_done();
    parked.tracking.as_mut().unwrap().status = LocalStatus::Parked;
    assert!(only(&snap(vec![parked]), RuleId::IntegrateReady).is_empty());
    let mut integrated = active_done();
    integrated.tracking.as_mut().unwrap().status = LocalStatus::Integrated;
    assert!(only(&snap(vec![integrated]), RuleId::IntegrateReady).is_empty());
}

fn prepped(state: PrepState) -> PrepSnapshot {
    PrepSnapshot {
        at: 5,
        repos: vec![PrepRepo {
            repo: "web".into(),
            state,
            files: vec!["a.php".into(), "b.php".into()],
            reason: Some("the integration worktree carries the test overlay".into()),
        }],
    }
}

#[test]
fn a_conflict_from_the_last_prepare_is_surfaced_and_blocks_nothing_else_silently() {
    let mut t = active_done();
    t.prep = Some(prepped(PrepState::Conflict));
    let s = snap(vec![t]);
    let blocked = only(&s, RuleId::IntegrationBlocked);
    assert_eq!(blocked.len(), 1);
    assert_eq!(
        blocked[0].action,
        SuggestedAction::ResolveConflict {
            ticket: key("PROJ-1"),
            repo: "web".into(),
            files: vec!["a.php".into(), "b.php".into()]
        }
    );
    assert!(blocked[0].informational);
    assert!(blocked[0].reason.contains("a.php, b.php"));
    // The conflict outranks the retry.
    let all = suggest(&s, NOW);
    assert_eq!(all[0].rule, RuleId::IntegrationBlocked);
    assert_eq!(all[1].rule, RuleId::IntegrateReady);
    assert!(all[1].reason.contains("blocked"));
}

#[test]
fn ready_or_finished_prepares_are_not_conflicts() {
    for state in [
        PrepState::Ready,
        PrepState::UpToDate,
        PrepState::AlreadyPushed,
    ] {
        let mut t = active_done();
        t.prep = Some(prepped(state));
        assert!(only(&snap(vec![t]), RuleId::IntegrationBlocked).is_empty());
    }
}

#[test]
fn a_blocked_repo_is_surfaced_with_its_reason() {
    let mut t = active_done();
    t.prep = Some(prepped(PrepState::Blocked));
    let list = only(&snap(vec![t]), RuleId::IntegrationBlocked);
    assert!(list[0].reason.contains("test overlay"));
}

#[test]
fn a_fix_after_alpha_suggests_activating_again_and_never_while_something_is_active() {
    let mut t = mine("PROJ-1", LocalStatus::Integrated, 0);
    t.remerge_repos = vec!["web".into()];
    merged(&mut t, "web", 100);
    let list = only(&snap(vec![t.clone()]), RuleId::RemergeNeeded);
    assert_eq!(
        list[0].action,
        SuggestedAction::Activate {
            ticket: key("PROJ-1"),
            baseline: None
        }
    );
    assert!(list[0].reason.contains("web"));

    let busy = snap(vec![t.clone(), mine("PROJ-2", LocalStatus::Active, 1)]);
    assert!(only(&busy, RuleId::RemergeNeeded).is_empty());

    t.remerge_repos.clear();
    assert!(only(&snap(vec![t]), RuleId::RemergeNeeded).is_empty());
}

#[test]
fn once_reactivated_the_remerge_is_an_integrate_suggestion() {
    let mut t = active_done();
    t.remerge_repos = vec!["web".into()];
    let list = only(&snap(vec![t]), RuleId::IntegrateReady);
    assert!(list[0].reason.contains("again"));
    assert_eq!(list[0].facts["remerge_repos"], json!(["web"]));
}

// ---------------------------------------------------------------- overlay

#[test]
fn a_leftover_overlay_is_reverted_but_an_active_tickets_is_normal() {
    let mut left = mine("PROJ-1", LocalStatus::Integrated, 0);
    left.overlays = vec!["web".into()];
    let list = only(&snap(vec![left]), RuleId::RevertOverlay);
    assert_eq!(
        list[0].action,
        SuggestedAction::RevertOverlay {
            ticket: key("PROJ-1"),
            repos: vec!["web".into()]
        }
    );
    assert_eq!(list[0].priority, Priority(985));

    let mut active = active_done();
    active.overlays = vec!["web".into()];
    assert!(only(&snap(vec![active.clone()]), RuleId::RevertOverlay).is_empty());

    // An overlay named as the reason a prepare was blocked.
    active.prep = Some(prepped(PrepState::Blocked));
    assert_eq!(only(&snap(vec![active]), RuleId::RevertOverlay).len(), 1);
}

// -------------------------------------------------------------- pipelines

fn integrated() -> TicketSnapshot {
    let mut t = mine("PROJ-1", LocalStatus::Integrated, 0);
    merged(&mut t, "web", 100);
    t
}

#[test]
fn a_failed_pipeline_offers_the_log_and_a_gateway_rerun() {
    let mut t = integrated();
    t.deploy = vec![deploy("web", DeployState::Failed)];
    let s = snap(vec![t]);
    let list = only(&s, RuleId::DeployFailed);
    assert_eq!(list.len(), 2);
    let kinds: Vec<&str> = list.iter().map(|x| x.action.kind()).collect();
    assert!(kinds.contains(&"open_pipeline") && kinds.contains(&"gateway"));
    let rerun = list.iter().find(|x| x.action.kind() == "gateway").unwrap();
    assert_eq!(
        rerun.action,
        SuggestedAction::Gateway(Action::RerunPipeline {
            repo: "acme/web".into(),
            run_id: "{web-run}".into()
        })
    );
    assert_eq!(rerun.level, ExecutionLevel::ConfirmedExternal);
    assert_eq!(suggest(&s, NOW)[0].priority, Priority(1000));
    assert!(only(&s, RuleId::DeployWaiting).is_empty());
}

#[test]
fn running_pipelines_are_only_a_waiting_note() {
    let mut t = integrated();
    t.deploy = vec![
        deploy("web", DeployState::Running),
        deploy("api", DeployState::Deployed),
    ];
    let list = only(&snap(vec![t]), RuleId::DeployWaiting);
    assert_eq!(list.len(), 1);
    assert!(list[0].informational);
    assert_eq!(list[0].level, ExecutionLevel::Automatic);
    assert!(list[0].reason.contains("web (running)"));
    assert!(!list[0].reason.contains("api"));
}

#[test]
fn nothing_is_watched_before_a_push_or_once_everything_deployed() {
    let mut unpushed = mine("PROJ-1", LocalStatus::Active, 0);
    unpushed.deploy = vec![deploy("web", DeployState::Pending)];
    assert!(only(&snap(vec![unpushed]), RuleId::DeployWaiting).is_empty());

    let mut t = integrated();
    t.deploy = vec![deploy("web", DeployState::Deployed)];
    let s = snap(vec![t]);
    assert!(only(&s, RuleId::DeployWaiting).is_empty());
    assert!(only(&s, RuleId::DeployFailed).is_empty());
}

// --------------------------------------------------------------- announce

#[test]
fn all_deployed_suggests_the_comment_draft_once() {
    let mut t = integrated();
    t.deploy = vec![deploy("web", DeployState::Deployed)];
    let list = only(&snap(vec![t.clone()]), RuleId::ComposeDeployComment);
    assert_eq!(
        list[0].action,
        SuggestedAction::ComposeDeployComment {
            ticket: key("PROJ-1"),
            partial: false
        }
    );

    // Not every repo deployed.
    let mut half = t.clone();
    half.deploy.push(deploy("api", DeployState::Running));
    assert!(only(&snap(vec![half]), RuleId::ComposeDeployComment).is_empty());

    // A draft exists.
    let mut drafted = t.clone();
    drafted.comment_draft = Some(DraftInfo {
        id: 3,
        body: "b".into(),
        created_at: 200,
    });
    assert!(only(&snap(vec![drafted]), RuleId::ComposeDeployComment).is_empty());

    // Already announced after the newest merge...
    let mut posted = t.clone();
    posted.comment_posted_at = Some(150);
    assert!(only(&snap(vec![posted.clone()]), RuleId::ComposeDeployComment).is_empty());
    // ...but a newer merge needs a new comment.
    merged(&mut posted, "web", 300);
    assert_eq!(
        only(&snap(vec![posted]), RuleId::ComposeDeployComment).len(),
        1
    );
}

#[test]
fn repos_without_pipeline_tracking_make_the_comment_partial() {
    let mut t = integrated();
    t.deploy = vec![
        deploy("web", DeployState::Deployed),
        deploy("docs", DeployState::Untracked),
    ];
    let list = only(&snap(vec![t.clone()]), RuleId::ComposeDeployComment);
    assert_eq!(
        list[0].action,
        SuggestedAction::ComposeDeployComment {
            ticket: key("PROJ-1"),
            partial: true
        }
    );
    t.deploy = vec![deploy("docs", DeployState::Untracked)];
    assert!(only(&snap(vec![t]), RuleId::ComposeDeployComment).is_empty());
}

#[test]
fn an_unposted_draft_is_posted_through_the_gateway_and_a_posted_one_never_again() {
    let mut t = integrated();
    t.comment_draft = Some(DraftInfo {
        id: 4,
        body: "Deployed.".into(),
        created_at: 200,
    });
    let list = only(&snap(vec![t.clone()]), RuleId::PostDeployComment);
    assert_eq!(
        list[0].action,
        SuggestedAction::Gateway(Action::PostJiraComment {
            ticket: key("PROJ-1"),
            body: "Deployed.".into()
        })
    );
    assert_eq!(list[0].facts["draft_id"], 4);
    assert_eq!(list[0].level, ExecutionLevel::ConfirmedExternal);

    // The loader only reports open drafts: once posted there is none.
    t.comment_draft = None;
    t.comment_posted_at = Some(300);
    assert!(only(&snap(vec![t]), RuleId::PostDeployComment).is_empty());
}

#[test]
fn the_transition_follows_the_posted_comment_once() {
    let mut t = integrated();
    t.comment_posted_at = Some(300);
    let s = snap(vec![t.clone()]);
    let list = only(&s, RuleId::TransitionAlpha);
    assert_eq!(
        list[0].action,
        SuggestedAction::Gateway(Action::TransitionJira {
            ticket: key("PROJ-1"),
            to_status: "Alpha Testing".into()
        })
    );

    // No comment yet.
    let mut none = t.clone();
    none.comment_posted_at = None;
    assert!(only(&snap(vec![none]), RuleId::TransitionAlpha).is_empty());
    // Already transitioned after the comment.
    let mut moved = t.clone();
    moved.transition_posted_at = Some(300);
    assert!(only(&snap(vec![moved]), RuleId::TransitionAlpha).is_empty());
    // A transition older than the (new) comment does not count.
    let mut older = t.clone();
    older.transition_posted_at = Some(200);
    assert_eq!(only(&snap(vec![older]), RuleId::TransitionAlpha).len(), 1);
    // Jira already shows alpha, UAT or done.
    for status in ["Alpha Testing", "UAT", "Done"] {
        let mut j = t.clone();
        j.jira_status = Some(status.into());
        assert!(
            only(&snap(vec![j]), RuleId::TransitionAlpha).is_empty(),
            "{status}"
        );
    }
}

// ---------------------------------------------------------------- returned

fn mention(id: &str, author: &str, at: i64) -> Mention {
    Mention {
        comment_id: id.into(),
        author: author.into(),
        author_account: author.into(),
        text: format!("hey {id}"),
        created_at: at,
    }
}

fn returned() -> TicketSnapshot {
    let mut t = mine("PROJ-1", LocalStatus::Integrated, 0);
    t.jira_status = Some("Returned".into());
    t.last_handled_at = Some(500);
    t.mentions = vec![mention("c1", "Sam", 400), mention("c2", "Sam", 600)];
    t
}

#[test]
fn a_returned_ticket_surfaces_only_for_a_new_mention() {
    let list = only(&snap(vec![returned()]), RuleId::ReturnedMention);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].facts["comment_id"], "c2");
    assert!(list[0].reason.contains("hey c2"));
    assert_eq!(list[0].priority, Priority(840));

    // Only older mentions.
    let mut old = returned();
    old.mentions = vec![mention("c1", "Sam", 400)];
    assert!(only(&snap(vec![old]), RuleId::ReturnedMention).is_empty());
    // Handled after the mention.
    let mut handled = returned();
    handled.last_handled_at = Some(700);
    assert!(only(&snap(vec![handled]), RuleId::ReturnedMention).is_empty());
    // My own comment.
    let mut own = returned();
    own.mentions = vec![mention("c3", "me", 900)];
    assert!(only(&snap(vec![own]), RuleId::ReturnedMention).is_empty());
    // Not returned.
    let mut alpha = returned();
    alpha.jira_status = Some("Alpha Testing".into());
    assert!(only(&snap(vec![alpha]), RuleId::ReturnedMention).is_empty());
}

#[test]
fn an_untracked_returned_ticket_with_a_mention_is_surfaced() {
    let mut t = TicketSnapshot::new(key("PROJ-5"));
    t.jira_status = Some("Returned".into());
    t.mentions = vec![mention("c1", "Sam", 10)];
    let list = only(&snap(vec![t]), RuleId::ReturnedMention);
    assert_eq!(list.len(), 1);
    assert!(only(&snap(vec![]), RuleId::ClaimNew).is_empty());
}

// ------------------------------------------------------------------ approval

fn signed_off() -> TicketSnapshot {
    let mut t = integrated();
    t.jira_status = Some("UAT".into());
    t.testing_complete = true;
    t.prs = vec![
        open_pr("web", "acme/web", 7, "develop"),
        open_pr("api", "acme/api", 8, "develop"),
    ];
    t
}

#[test]
fn testing_complete_offers_each_unapproved_open_pr() {
    let list = only(&snap(vec![signed_off()]), RuleId::ApprovePrs);
    assert_eq!(list.len(), 2);
    assert_eq!(
        list[0].action,
        SuggestedAction::Gateway(Action::ApprovePr {
            repo: "acme/api".into(),
            pr: 8
        })
    );
}

#[test]
fn approval_skips_approved_own_closed_and_untested_prs() {
    let mut t = signed_off();
    t.prs[0].pr.reviewers = vec![Reviewer {
        account: "me".into(),
        approved: true,
        changes_requested: false,
    }];
    t.prs[1].pr.author = "me".into();
    assert!(only(&snap(vec![t.clone()]), RuleId::ApprovePrs).is_empty());

    let mut other = signed_off();
    other.prs[0].pr.reviewers = vec![Reviewer {
        account: "sam".into(),
        approved: true,
        changes_requested: false,
    }];
    assert_eq!(only(&snap(vec![other]), RuleId::ApprovePrs).len(), 2);

    let mut merged_pr = signed_off();
    merged_pr.prs[0].pr.state = PrState::Merged;
    assert_eq!(only(&snap(vec![merged_pr]), RuleId::ApprovePrs).len(), 1);

    let mut untested = signed_off();
    untested.testing_complete = false;
    assert!(only(&snap(vec![untested]), RuleId::ApprovePrs).is_empty());
}

#[test]
fn testing_complete_needs_alpha_and_a_signed_off_status() {
    let st = StatusNames::default();
    // The uat status is still testing, not sign-off.
    assert!(!is_testing_complete(&st, Some("UAT"), true));
    assert!(is_testing_complete(&st, Some("done"), true));
    assert!(!is_testing_complete(&st, Some("UAT"), false));
    assert!(!is_testing_complete(&st, Some("Alpha Testing"), true));
    assert!(!is_testing_complete(&st, None, true));
    let custom = StatusNames {
        done: vec!["Closed".into()],
        signed_off: vec!["Closed".into()],
        ..StatusNames::default()
    };
    assert!(is_testing_complete(&custom, Some("Closed"), true));
    assert!(!is_testing_complete(&custom, Some("Done"), true));
    let uat_passed = StatusNames::from(&crate::config::JiraStatuses {
        signed_off: vec!["UAT Passed".into()],
        ..Default::default()
    });
    assert!(is_testing_complete(&uat_passed, Some("uat passed"), true));
    assert!(!is_testing_complete(&uat_passed, Some("UAT"), true));
    assert!(!is_testing_complete(&uat_passed, Some("Done"), true));
}

// -------------------------------------------------------- adapters, purity

#[test]
fn an_unavailable_writer_replaces_the_gateway_suggestion_with_a_note() {
    let mut t = integrated();
    t.comment_draft = Some(DraftInfo {
        id: 4,
        body: "x".into(),
        created_at: 1,
    });
    let mut s = snap(vec![t]);
    s.writers.jira = Availability::Unavailable("acli is not installed".into());
    let all = suggest(&s, NOW);
    assert!(
        !all.iter()
            .any(|x| matches!(x.action, SuggestedAction::Gateway(_)))
    );
    let note = all
        .iter()
        .find(|x| x.rule == RuleId::AdapterUnavailable)
        .unwrap();
    assert!(note.informational && note.level == ExecutionLevel::Automatic);
    assert!(
        note.reason.contains("acli is not installed")
            && note.reason.contains("post the deploy comment")
    );
    assert!(note.priority.0 <= 299);
}

#[test]
fn unavailable_code_host_notes_collapse_per_ticket() {
    let mut s = snap(vec![signed_off()]);
    s.writers.code_host = Availability::Unavailable("bkt not logged in".into());
    let notes: Vec<_> = suggest(&s, NOW)
        .into_iter()
        .filter(|x| x.rule == RuleId::AdapterUnavailable)
        .collect();
    assert_eq!(notes.len(), 1);
}

#[test]
fn suggest_is_deterministic() {
    let mut t = integrated();
    t.deploy = vec![deploy("web", DeployState::Failed)];
    let s = snap(vec![
        t,
        pool("PROJ-9", Some("High")),
        mine("PROJ-2", LocalStatus::Claimed, 3),
    ]);
    assert_eq!(suggest(&s, NOW), suggest(&s.clone(), NOW));
}

// -------------------------------------------------------------- ordering

#[test]
fn priority_bands_order_blocking_review_in_flight_queue_housekeeping() {
    let mut failed = integrated();
    failed.deploy = vec![deploy("web", DeployState::Failed)];
    let mut review = mine("PROJ-2", LocalStatus::Claimed, 1);
    review.prs = vec![open_pr("web", "acme/web", 9, "develop")];
    let mut flight = mine("PROJ-3", LocalStatus::Active, 2);
    flight.checklist = Checklist { total: 1, done: 1 };
    flight.touched_repos = vec!["web".into()];
    let mut s = snap(vec![failed, review, flight, pool("PROJ-8", None)]);
    s.sync.expected = vec!["jira".into()];
    let rules = rules_of(&suggest(&s, NOW));
    assert_eq!(
        rules,
        [
            RuleId::SyncStale,
            RuleId::DeployFailed,
            RuleId::DeployFailed,
            RuleId::StartReview,
            RuleId::IntegrateReady,
            RuleId::ClaimNew,
        ]
    );
}

#[test]
fn housekeeping_ranks_below_the_queue() {
    let mut stale = mine("PROJ-4", LocalStatus::Parked, 3);
    stale.tracking.as_mut().unwrap().updated_at = -2_000_000;
    let all = suggest(&snap(vec![stale, pool("PROJ-8", None)]), NOW);
    assert_eq!(
        rules_of(&all),
        [
            RuleId::ActivateReviewed,
            RuleId::ClaimNew,
            RuleId::StaleTicket
        ]
    );
}

#[test]
fn a_hotfix_review_request_outranks_a_normal_one_and_ties_with_blocking() {
    let mut normal = mine("PROJ-1", LocalStatus::Claimed, 0);
    normal.prs = vec![open_pr("web", "acme/web", 1, "develop")];
    let mut hot = mine("PROJ-2", LocalStatus::Claimed, 1);
    hot.kind = TicketKind::Hotfix;
    hot.prs = vec![open_pr("web", "acme/web", 2, "master")];
    let mut failed = integrated();
    failed.key = key("PROJ-3");
    failed.tracking.as_mut().unwrap().manual_order = 2;
    failed.deploy = vec![deploy("web", DeployState::Failed)];
    let all = suggest(&snap(vec![normal, hot, failed]), NOW);
    let order: Vec<(&str, u32)> = all
        .iter()
        .map(|x| (x.ticket.as_ref().unwrap().as_str(), x.priority.0))
        .collect();
    assert_eq!(order[0], ("PROJ-2", 1020));
    assert_eq!(order[1], ("PROJ-3", 1000));
    assert_eq!(order.last().unwrap(), &("PROJ-1", 820));
}

#[test]
fn ties_break_by_manual_order_then_key() {
    let mut a = mine("PROJ-9", LocalStatus::Claimed, 0);
    let mut b = mine("PROJ-1", LocalStatus::Claimed, 1);
    a.prs.clear();
    b.prs.clear();
    let all = suggest(&snap(vec![b, a]), NOW);
    let keys: Vec<&str> = all
        .iter()
        .map(|x| x.ticket.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(keys, ["PROJ-9", "PROJ-1"], "manual order, not key order");

    // Equal manual order (should not happen) falls back to the key.
    let x = mine("PROJ-9", LocalStatus::Claimed, 0);
    let y = mine("PROJ-1", LocalStatus::Claimed, 0);
    let all = suggest(&snap(vec![x, y]), NOW);
    assert_eq!(all[0].ticket, Some(key("PROJ-1")));
}

#[test]
fn claims_follow_tracked_work_in_the_same_priority_by_jira_rank() {
    let all = suggest(
        &snap(vec![
            pool("PROJ-2", Some("High")),
            pool("PROJ-1", Some("Low")),
        ]),
        NOW,
    );
    assert_eq!(all[0].ticket, Some(key("PROJ-2")));
}

// -------------------------------------------------------------------- ids

#[test]
fn ids_are_stable_and_distinguish_subjects() {
    let a = Suggestion::make_id(Some(&key("PROJ-1")), RuleId::ApprovePrs, "acme/web#7");
    assert_eq!(a, "PROJ-1:approve_prs:acme/web#7");
    assert_eq!(
        Suggestion::make_id(None, RuleId::SyncStale, ""),
        "-:sync_stale:-"
    );

    let s = snap(vec![signed_off()]);
    let ids: Vec<String> = suggest(&s, NOW).into_iter().map(|x| x.id).collect();
    let again: Vec<String> = suggest(&s, NOW + 5).into_iter().map(|x| x.id).collect();
    assert_eq!(ids, again);
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn facts_never_depend_on_the_clock() {
    let mut a = mine("PROJ-1", LocalStatus::Active, 0);
    a.tracking.as_mut().unwrap().updated_at = 0;
    let s = snap(vec![a]);
    let early = suggest(&s, 6 * 86_400);
    let late = suggest(&s, 9 * 86_400);
    assert_eq!(early[0].facts, late[0].facts);
    assert_eq!(early[0].facts_hash(), late[0].facts_hash());
}

#[test]
fn the_facts_hash_ignores_key_order_and_reacts_to_values() {
    assert_eq!(
        facts_hash(&json!({ "a": 1, "b": [1, 2], "c": { "x": 1, "y": 2 } })),
        facts_hash(&json!({ "c": { "y": 2, "x": 1 }, "b": [1, 2], "a": 1 }))
    );
    assert_ne!(
        facts_hash(&json!({ "a": 1 })),
        facts_hash(&json!({ "a": 2 }))
    );
    assert_ne!(facts_hash(&json!([1, 2])), facts_hash(&json!([2, 1])));
    // Pinned: stored hashes must keep their meaning.
    assert_eq!(facts_hash(&json!({ "a": 1 })), "9c3e82dd6fcae8b1");
}

// ---------------------------------------------------------------- responses

fn one() -> Suggestion {
    suggest(&snap(vec![pool("PROJ-1", None)]), NOW).remove(0)
}

fn answer(
    s: &Suggestion,
    kind: ResponseKind,
    until: Option<i64>,
) -> BTreeMap<String, SuggestionResponse> {
    let r = make_response(s, kind, Some("because".into()), until, NOW);
    BTreeMap::from([(r.suggestion_id.clone(), r)])
}

#[test]
fn a_dismissal_hides_until_the_facts_change() {
    let s = one();
    let responses = answer(&s, ResponseKind::Dismissed, None);
    assert!(apply_responses(vec![s.clone()], &responses, NOW + 1).is_empty());
    assert_eq!(
        classify(&s, responses.get(&s.id), NOW),
        ResponseState::Dismissed
    );

    let mut changed = s.clone();
    changed.facts = json!({ "ticket": "PROJ-1", "queue_position": 2 });
    assert_eq!(
        apply_responses(vec![changed.clone()], &responses, NOW).len(),
        1
    );
    assert_eq!(
        classify(&changed, responses.get(&s.id), NOW),
        ResponseState::Resurfaced
    );
}

#[test]
fn a_snooze_returns_at_its_end_regardless_of_facts() {
    let s = one();
    let responses = answer(&s, ResponseKind::Snoozed, Some(NOW + 100));
    assert!(apply_responses(vec![s.clone()], &responses, NOW + 99).is_empty());
    assert_eq!(
        classify(&s, responses.get(&s.id), NOW + 99),
        ResponseState::Snoozed
    );
    assert_eq!(
        apply_responses(vec![s.clone()], &responses, NOW + 100).len(),
        1
    );
    let mut changed = s.clone();
    changed.facts = json!({ "different": true });
    assert!(apply_responses(vec![changed], &responses, NOW + 50).is_empty());
}

#[test]
fn done_hides_while_the_facts_are_the_same() {
    let s = one();
    let responses = answer(&s, ResponseKind::Done, None);
    assert_eq!(classify(&s, responses.get(&s.id), NOW), ResponseState::Done);
    assert!(apply_responses(vec![s.clone()], &responses, NOW).is_empty());
    let mut changed = s;
    changed.facts = json!({});
    assert_eq!(apply_responses(vec![changed], &responses, NOW).len(), 1);
}

#[test]
fn unanswered_and_unrelated_suggestions_stay() {
    let s = one();
    let other = suggest(&snap(vec![pool("PROJ-2", None)]), NOW).remove(0);
    let responses = answer(&other, ResponseKind::Dismissed, None);
    assert_eq!(apply_responses(vec![s], &responses, NOW).len(), 1);
}

#[test]
fn next_actions_applies_the_filter_after_ranking() {
    let s = snap(vec![pool("PROJ-1", Some("High")), pool("PROJ-2", None)]);
    let first = suggest(&s, NOW).remove(0);
    let responses = answer(&first, ResponseKind::Dismissed, None);
    let visible = next_actions(&s, &responses, NOW);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].ticket, Some(key("PROJ-2")));
}

#[test]
fn durations_parse() {
    assert_eq!(parse_duration("90s"), Some(90));
    assert_eq!(parse_duration("2h"), Some(7200));
    assert_eq!(parse_duration("1d"), Some(86_400));
    assert_eq!(parse_duration("2w"), Some(14 * 86_400));
    for bad in ["", "h", "0h", "2", "2x", "-1h", "1.5h", "2 h"] {
        assert_eq!(parse_duration(bad), None, "{bad:?}");
    }
}

// -------------------------------------------------- types keep writes gated

#[test]
fn only_the_gateway_variant_carries_an_external_write() {
    // Every level-ConfirmedExternal action is `Gateway(_)`, every other variant is not.
    let key: TicketKey = key("PROJ-1");
    let samples = [
        SuggestedAction::Sync,
        SuggestedAction::Claim {
            ticket: key.clone(),
        },
        SuggestedAction::Park {
            ticket: key.clone(),
        },
        SuggestedAction::RunIntegrationPrepare {
            ticket: key.clone(),
        },
        SuggestedAction::OpenTicket {
            ticket: key.clone(),
            url: None,
        },
        SuggestedAction::Gateway(Action::ApprovePr {
            repo: "r".into(),
            pr: 1,
        }),
    ];
    for a in samples {
        assert_eq!(
            a.level() == ExecutionLevel::ConfirmedExternal,
            matches!(a, SuggestedAction::Gateway(_))
        );
    }
}

#[test]
fn the_engine_sources_never_touch_writers_or_confirmations() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/next");
    for file in [
        "rules.rs",
        "engine.rs",
        "snapshot.rs",
        "load.rs",
        "model.rs",
        "responses.rs",
    ] {
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        // Strip comments so the docs may mention them.
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for banned in [
            "TicketWriter",
            "CodeHostWriter",
            concat!("build_ticket", "_writer"),
            concat!("build_code_host", "_writer"),
            ".confirm()",
            ".execute(",
            "gateway::Confirmed",
            "Gateway::draft",
        ] {
            assert!(!code.contains(banned), "{file} uses {banned}");
        }
    }
}

#[test]
fn rule_ids_are_unique_and_listed() {
    let mut names: Vec<&str> = RuleId::ALL.iter().map(|r| r.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), RuleId::ALL.len());
    // Pr helper stays in sync with the model used above.
    let _: Pr = pr("a/b", 1, "x", "develop");
}

// ------------------------------------------------- independent review findings

#[test]
fn a_dismissed_claim_is_not_resurfaced_when_the_queue_around_it_changes() {
    // Facts must hold what changes with the situation of the ticket, not its place in a
    // queue that other tickets enter and leave all day.
    let before = snap(vec![pool("PROJ-2", Some("Low"))]);
    let claim = only(&before, RuleId::ClaimNew).remove(0);
    let responses = answer(&claim, ResponseKind::Dismissed, None);

    let after = snap(vec![
        pool("PROJ-2", Some("Low")),
        pool("PROJ-1", Some("High")),
    ]);
    let listed = next_actions(&after, &responses, NOW);
    assert!(
        listed.iter().all(|s| s.id != claim.id),
        "a new ticket in the queue brought the dismissed claim back"
    );
}

#[test]
fn my_own_comment_is_recognised_by_account_id_not_by_display_name() {
    // The loader knows the comment author's display name and account id; the configured
    // identity is an account id, so comparing it with the name never matched.
    let mut t = returned();
    t.mentions = vec![Mention {
        comment_id: "c3".into(),
        author: "Display Name".into(),
        author_account: "me".into(),
        text: "note to self".into(),
        created_at: 900,
    }];
    assert!(only(&snap(vec![t]), RuleId::ReturnedMention).is_empty());

    // Someone whose display name happens to be my account id is somebody else.
    let mut t = returned();
    t.mentions = vec![Mention {
        comment_id: "c4".into(),
        author: "me".into(),
        author_account: "someone-else".into(),
        text: "ping".into(),
        created_at: 900,
    }];
    assert_eq!(only(&snap(vec![t]), RuleId::ReturnedMention).len(), 1);
}

#[test]
fn a_returned_or_finished_ticket_is_not_pushed_along_the_local_flow() {
    // GOAL.md: a Returned ticket is ignored unless you are mentioned or it returns to
    // Review. Rules that move a ticket towards review, test or integration stay quiet.
    for jira in ["Returned", "returned ", "Done"] {
        let mut claimed = mine("PROJ-1", LocalStatus::Claimed, 0);
        claimed.jira_status = Some(jira.into());
        let mut reviewing = mine("PROJ-2", LocalStatus::Reviewing, 1);
        reviewing.jira_status = Some(jira.into());
        let mut reviewed_t = reviewed("PROJ-3", LocalStatus::Reviewing, 2);
        reviewed_t.jira_status = Some(jira.into());
        let mut active = active_done();
        active.jira_status = Some(jira.into());

        // (Nothing may be active for the activation rule to speak at all.)
        let mut all = suggest(&snap(vec![claimed, reviewing, reviewed_t]), NOW);
        all.extend(suggest(&snap(vec![active]), NOW));
        let noisy: Vec<_> = all
            .iter()
            .filter(|s| {
                matches!(
                    s.rule,
                    RuleId::StartReview
                        | RuleId::FinishReview
                        | RuleId::ActivateReviewed
                        | RuleId::IntegrateReady
                )
            })
            .map(|s| s.id.clone())
            .collect();
        assert!(noisy.is_empty(), "{jira}: {noisy:?}");
    }

    // Back in Review it is an ordinary ticket again.
    let mut claimed = mine("PROJ-1", LocalStatus::Claimed, 0);
    claimed.jira_status = Some("In Review".into());
    assert_eq!(only(&snap(vec![claimed]), RuleId::StartReview).len(), 1);
}

#[test]
fn only_external_writes_are_remembered_as_done() {
    // A local action changes the store at once. Remembering it as done would hide the
    // same suggestion when it recurs with identical facts (parked a second time, a
    // review restarted after new commits).
    let t = key("PROJ-1");
    for local in [
        SuggestedAction::Claim { ticket: t.clone() },
        SuggestedAction::StartReview { ticket: t.clone() },
        SuggestedAction::MarkReviewed { ticket: t.clone() },
        SuggestedAction::Activate {
            ticket: t.clone(),
            baseline: None,
        },
        SuggestedAction::Park { ticket: t.clone() },
        SuggestedAction::RunIntegrationPrepare { ticket: t.clone() },
        SuggestedAction::Sync,
    ] {
        assert!(!exec::records_done(&local), "{}", local.kind());
    }
    assert!(exec::records_done(&SuggestedAction::Gateway(
        Action::ApprovePr {
            repo: "acme/web".into(),
            pr: 1
        }
    )));
}

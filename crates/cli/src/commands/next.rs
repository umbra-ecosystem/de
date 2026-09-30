//! `de next`: the ranked suggestions of the next-action engine, and `dismiss`, `snooze` and
//! `do` to answer or carry out one.
//!
//! Thin on purpose: the ranking lives in `de_core::next` (pure), executing reuses the core
//! executors and the existing `ship`/`ticket`/`sync` commands. Rendering is in pure
//! functions (`render_list`, `render_json`, `describe_action`) which the tests cover.
//!
//! **No flag skips a confirmation.** Local steps ask with `dialoguer::Confirm`; external ones
//! show the gateway's exact preview and ask again; without a terminal both refuse.

use de_core::{
    activation::WorkspaceRepo,
    config::Config,
    domain::{BaselineChoice, TicketKey},
    gateway::Gateway,
    next::{
        ExecutionLevel, LoadContext, ResponseState, Snapshot, SuggestedAction, Suggestion,
        annotate, exec, parse_duration, suggest,
    },
    overlay::ProcessRunner,
    store::{
        Kind, Store,
        suggestion_responses::{self, ResponseKind},
    },
    sync::HostedRepo,
};
use eyre::{Context, bail, eyre};
use serde_json::{Value, json};

use super::{
    ship::{confirm, render_preview},
    ticket::{ask_baseline, render_activation, render_deactivation},
};
use crate::{types::Slug, utils::ui::UserInterface, workspace::Workspace};

/// How many suggestions `de next` shows without `--all`.
pub const DEFAULT_LIMIT: usize = 5;

/// The version of the `--json` shape. Bump it on any incompatible change.
pub const JSON_VERSION: u32 = 1;

fn now() -> eyre::Result<i64> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .wrap_err("The system clock is before 1970")?;
    Ok(since_epoch.as_secs() as i64)
}

// ------------------------------------------------------------ pure: render

fn level_label(level: ExecutionLevel) -> &'static str {
    match level {
        ExecutionLevel::Automatic => "runs on its own",
        ExecutionLevel::LocalOneClick => "one click, local",
        ExecutionLevel::ConfirmedExternal => "asks first, external",
    }
}

/// The text of `de next`: at most [`DEFAULT_LIMIT`] visible suggestions, or with `all` every
/// suggestion including the dismissed, snoozed and finished ones (marked).
pub fn render_list(items: &[(Suggestion, ResponseState)], all: bool) -> Vec<String> {
    let shown: Vec<&(Suggestion, ResponseState)> = items
        .iter()
        .filter(|(_, state)| all || state.is_visible())
        .collect();
    if shown.is_empty() {
        return vec!["Nothing to do right now.".into()];
    }
    let limit = if all { shown.len() } else { DEFAULT_LIMIT };
    let mut lines = Vec::new();
    for (i, (s, state)) in shown.iter().take(limit).enumerate() {
        let mark = match state {
            ResponseState::Open => String::new(),
            other => format!("  ({})", other.as_str()),
        };
        lines.push(format!(
            "{:>2}. [{}] {}  {}{mark}",
            i + 1,
            s.priority,
            s.ticket.as_ref().map_or("-", TicketKey::as_str),
            s.reason
        ));
        lines.push(format!(
            "    {} | {} | {}",
            level_label(s.level),
            s.rule,
            s.id
        ));
    }
    if shown.len() > limit {
        lines.push(format!(
            "    ... {} more (de next --all)",
            shown.len() - limit
        ));
    }
    lines
}

/// The machine-readable form the menubar app consumes. A contract: `version` is
/// [`JSON_VERSION`]. Every suggestion is listed (the GUI decides how many to show), in rank
/// order, with `state` telling open ones (`open`, `resurfaced`) from answered ones.
pub fn render_json(items: &[(Suggestion, ResponseState)], generated_at: i64) -> String {
    let suggestions: Vec<Value> = items
        .iter()
        .enumerate()
        .map(|(i, (s, state))| {
            json!({
                "rank": i + 1,
                "id": s.id,
                "ticket": s.ticket,
                "rule": s.rule,
                "priority": s.priority,
                "level": s.level,
                "informational": s.informational,
                "state": state.as_str(),
                "action": s.action,
                "reason": s.reason,
                "facts": s.facts,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({
        "version": JSON_VERSION,
        "generated_at": generated_at,
        "suggestions": suggestions,
    }))
    .expect("plain JSON always serialises")
}

/// What carrying the action out will do, in plain lines (shown before asking).
pub fn describe_action(action: &SuggestedAction) -> Vec<String> {
    match action {
        SuggestedAction::Sync => vec!["Read Jira and Bitbucket into the local cache.".into()],
        SuggestedAction::Claim { ticket } => vec![format!("Start tracking {ticket} locally.")],
        SuggestedAction::StartReview { ticket } => vec![format!(
            "Mark the review of {ticket} as started (clears an earlier reviewed marker)."
        )],
        SuggestedAction::MarkReviewed { ticket } => vec![format!(
            "Record {ticket} as reviewed, with the ticket branches' current tips."
        )],
        SuggestedAction::Activate { ticket, baseline } => vec![
            format!(
                "Activate {ticket}: switch repos to its branch (others to the {} baseline), stashing local changes.",
                baseline.unwrap_or(BaselineChoice::Base)
            ),
            "Applies the composer test overlay where needed and runs the rebuild tasks.".into(),
        ],
        SuggestedAction::ChooseHotfixBaseline { ticket } => vec![format!(
            "Choose the baseline for repos {ticket} does not touch, then activate it."
        )],
        SuggestedAction::Park { ticket } => vec![format!(
            "Park {ticket}: revert its test overlay and restore each repo, stashing uncommitted work."
        )],
        SuggestedAction::MarkDone { ticket } => vec![format!("Mark {ticket} as done.")],
        SuggestedAction::RunIntegrationPrepare { ticket } => vec![
            format!(
                "Prepare merging {ticket} into uat in temporary worktrees; nothing is pushed yet."
            ),
            "The push is previewed and needs its own confirmation.".into(),
        ],
        SuggestedAction::ComposeDeployComment { ticket, partial } => vec![format!(
            "Draft the deploy comment for {ticket}{} (saved locally, not posted).",
            if *partial { ", marked partial" } else { "" }
        )],
        SuggestedAction::RevertOverlay { ticket, repos } => vec![format!(
            "Revert the composer test overlay of {ticket} in {} and rebuild vendor/.",
            repos.join(", ")
        )],
        SuggestedAction::ResolveConflict { repo, files, .. } => {
            let mut lines = vec![format!("{repo} cannot be merged into uat as it is.")];
            if !files.is_empty() {
                lines.push(format!("Conflicting files: {}", files.join(", ")));
            }
            lines.push("Resolve it on the ticket branch, then integrate again.".into());
            lines
        }
        SuggestedAction::OpenPr { repo, pr, url } => vec![format!("{repo} #{pr}: {url}")],
        SuggestedAction::OpenPipeline { repo, run_id, url } => {
            vec![format!("{repo} pipeline {run_id}: {url}")]
        }
        SuggestedAction::OpenTicket { ticket, url } => {
            vec![format!(
                "{ticket}: {}",
                url.as_deref().unwrap_or("no URL cached")
            )]
        }
        SuggestedAction::Waiting { what, .. } => vec![format!("Waiting for {what}.")],
        SuggestedAction::AdapterUnavailable {
            adapter,
            detail,
            blocked,
        } => vec![format!(
            "Cannot {blocked}: {adapter} is not available ({detail})."
        )],
        SuggestedAction::Gateway(action) => action.preview().to_lines(),
    }
}

// ------------------------------------------------------------- gathering

struct Loaded {
    state: Store,
    cache: Store,
    config: Config,
    repos: Vec<WorkspaceRepo>,
    hosted: Vec<HostedRepo>,
    workspace_default_branch: Option<String>,
}

fn load_env(workspace: Option<Slug>) -> eyre::Result<Loaded> {
    let state = Store::open_default(Kind::State)?;
    let cache = Store::open_default(Kind::Cache)?;
    let config = Config::load()?;
    let workspace = match workspace {
        Some(name) => Some(
            Workspace::load_from_name(&name)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to load workspace")?
                .ok_or_else(|| eyre!("Workspace '{name}' not found"))?,
        ),
        None => Workspace::active()?,
    };
    let (repos, default_branch) = match &workspace {
        Some(w) => (
            WorkspaceRepo::from_workspace(w)
                .wrap_err("Failed to load the projects of the workspace")?,
            w.config().default_branch.clone(),
        ),
        None => (Vec::new(), None),
    };
    let hosted = HostedRepo::from_workspace_repos(&repos, &config);
    Ok(Loaded {
        state,
        cache,
        config,
        repos,
        hosted,
        workspace_default_branch: default_branch,
    })
}

fn rank(
    env: &Loaded,
    gateway: &Gateway<'_>,
    now: i64,
) -> eyre::Result<Vec<(Suggestion, ResponseState)>> {
    let snapshot = Snapshot::load(&LoadContext {
        state: &env.state,
        cache: &env.cache,
        config: &env.config,
        repos: &env.repos,
        hosted: &env.hosted,
        gateway,
    })?;
    let responses = suggestion_responses::latest_map(&env.state)?;
    Ok(annotate(suggest(&snapshot, now), &responses, now))
}

fn find(items: Vec<(Suggestion, ResponseState)>, id: &str) -> eyre::Result<Suggestion> {
    items
        .into_iter()
        .map(|(s, _)| s)
        .find(|s| s.id == id)
        .ok_or_else(|| {
            eyre!(
                "There is no suggestion '{id}' right now; run `de next --all` for the current ids"
            )
        })
}

// -------------------------------------------------------------- commands

/// `de next [--all] [--json]`.
pub fn list(workspace: Option<Slug>, all: bool, json: bool) -> eyre::Result<()> {
    let env = load_env(workspace)?;
    let gateway = Gateway::from_config(&env.state, &env.config);
    let now = now()?;
    let items = rank(&env, &gateway, now)?;

    if json {
        println!("{}", render_json(&items, now));
        return Ok(());
    }
    let ui = UserInterface::new();
    for line in render_list(&items, all) {
        ui.writeln(&line)?;
    }
    Ok(())
}

/// `de next dismiss <id> [--reason R]`.
pub fn dismiss(id: String, reason: Option<String>, workspace: Option<Slug>) -> eyre::Result<()> {
    let env = load_env(workspace)?;
    let gateway = Gateway::from_config(&env.state, &env.config);
    let now = now()?;
    let s = find(rank(&env, &gateway, now)?, &id)?;
    exec::respond(&env.state, &s, ResponseKind::Dismissed, reason, None, now)?;
    UserInterface::new().success_item(
        &format!("Dismissed {id}"),
        Some("It returns if its facts change materially."),
    )?;
    Ok(())
}

/// `de next snooze <id> --for <duration>`.
pub fn snooze(id: String, duration: String, workspace: Option<Slug>) -> eyre::Result<()> {
    let seconds = parse_duration(&duration)
        .ok_or_else(|| eyre!("'{duration}' is not a duration; use e.g. 30m, 2h, 1d, 1w"))?;
    let env = load_env(workspace)?;
    let gateway = Gateway::from_config(&env.state, &env.config);
    let now = now()?;
    let s = find(rank(&env, &gateway, now)?, &id)?;
    exec::respond(
        &env.state,
        &s,
        ResponseKind::Snoozed,
        None,
        Some(now + seconds),
        now,
    )?;
    UserInterface::new().success_item(&format!("Snoozed {id} for {duration}"), None)?;
    Ok(())
}

fn print_lines(ui: &UserInterface, lines: &[String]) -> eyre::Result<()> {
    for line in lines {
        ui.writeln(line)?;
    }
    Ok(())
}

/// `de next do <id>`.
pub fn do_it(id: String, workspace: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let env = load_env(workspace.clone())?;
    let gateway = Gateway::from_config(&env.state, &env.config);
    let started = now()?;
    let s = find(rank(&env, &gateway, started)?, &id)?;

    ui.heading(&s.reason)?;
    let ran = match &s.action {
        // Automatic: no prompt.
        SuggestedAction::Sync => {
            super::sync::sync(None, false)?;
            true
        }
        // Nothing to run: say what there is to say.
        a if a.is_informational() => {
            print_lines(&ui, &describe_action(a))?;
            false
        }
        SuggestedAction::Gateway(_) => {
            let prepared = exec::prepare_external(&gateway, &env.state, &env.config, &s, started)?;
            ui.new_line()?;
            print_lines(&ui, &render_preview(prepared.preview()))?;
            ui.new_line()?;
            if !confirm("Send exactly this?")? {
                ui.info_item("Cancelled; nothing was sent.")?;
                return Ok(());
            }
            let outcome = prepared.confirm().execute(&gateway, &env.state, now()?)?;
            ui.success_item(&format!("Done: {}", describe_outcome(&outcome)), None)?;
            true
        }
        SuggestedAction::RunIntegrationPrepare { ticket } => {
            print_lines(&ui, &describe_action(&s.action))?;
            if !confirm("Prepare the integration?")? {
                ui.info_item("Cancelled; nothing was changed.")?;
                return Ok(());
            }
            super::ship::integrate(ticket.clone(), false, workspace)?;
            true
        }
        SuggestedAction::ChooseHotfixBaseline { ticket } => {
            print_lines(&ui, &describe_action(&s.action))?;
            let baseline = ask_baseline()?;
            let action = SuggestedAction::Activate {
                ticket: ticket.clone(),
                baseline: Some(baseline),
            };
            run_local(&ui, &env, &action)?
        }
        local => {
            print_lines(&ui, &describe_action(local))?;
            if !confirm("Go ahead?")? {
                ui.info_item("Cancelled; nothing was changed.")?;
                return Ok(());
            }
            run_local(&ui, &env, local)?
        }
    };

    if ran {
        // Tied to the facts it was done for: it returns only if they change.
        exec::respond(&env.state, &s, ResponseKind::Done, None, None, now()?)?;
    }
    Ok(())
}

fn describe_outcome(outcome: &exec::ExternalOutcome) -> String {
    match outcome {
        exec::ExternalOutcome::CommentPosted(c) => format!("posted comment {}", c.id),
        exec::ExternalOutcome::Transitioned => "transitioned".into(),
        exec::ExternalOutcome::Executed(e) => format!("sent ({})", e.draft_id),
    }
}

fn run_local(ui: &UserInterface, env: &Loaded, action: &SuggestedAction) -> eyre::Result<bool> {
    let runner = ProcessRunner::new();
    let local = exec::LocalEnv {
        state: &env.state,
        cache: &env.cache,
        repos: &env.repos,
        hosted: &env.hosted,
        runner: &runner,
        workspace_default_branch: env.workspace_default_branch.clone(),
        fetch: false,
    };
    match exec::execute_local(&local, action, now()?)? {
        exec::LocalOutcome::Done(message) => ui.success_item(&message, None)?,
        exec::LocalOutcome::Activated(report) => {
            print_lines(ui, &render_activation(&report))?;
            for warning in &report.warnings {
                ui.warning_item(warning, None)?;
            }
            ui.success_item(&format!("Activated {}", report.ticket), None)?;
        }
        exec::LocalOutcome::Parked(report) => {
            print_lines(ui, &render_deactivation(&report))?;
            if !report.is_complete() {
                bail!(
                    "{} is still active because some repos could not be restored",
                    report.ticket
                );
            }
        }
        exec::LocalOutcome::Drafted(draft) => {
            ui.success_item(
                &format!("Draft {} saved (not posted)", draft.id),
                Some("The next suggestion posts it, after a preview and confirmation."),
            )?;
        }
        exec::LocalOutcome::Reverted(done) => {
            for (repo, _) in done {
                ui.success_item(&format!("Reverted the test overlay in {repo}"), None)?;
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_core::{gateway::Action, next::RuleId};

    fn key(s: &str) -> TicketKey {
        s.parse().unwrap()
    }

    fn claim() -> Suggestion {
        Suggestion::new(
            Some(&key("PROJ-1")),
            RuleId::ClaimNew,
            "-",
            SuggestedAction::Claim {
                ticket: key("PROJ-1"),
            },
            "PROJ-1 is in In Review and not claimed",
            json!({ "ticket": "PROJ-1", "queue_position": 1 }),
            480,
        )
    }

    fn approve() -> Suggestion {
        Suggestion::new(
            Some(&key("PROJ-2")),
            RuleId::ApprovePrs,
            "acme/web#7",
            SuggestedAction::Gateway(Action::ApprovePr {
                repo: "acme/web".into(),
                pr: 7,
            }),
            "PROJ-2 passed testing: approve acme/web #7",
            json!({ "repo": "acme/web", "pr": 7 }),
            690,
        )
    }

    fn sync() -> Suggestion {
        Suggestion::new(
            None,
            RuleId::SyncStale,
            "-",
            SuggestedAction::Sync,
            "Sources are not synced",
            json!({}),
            1200,
        )
    }

    #[test]
    fn the_text_lists_visible_suggestions_with_level_rule_and_id() {
        let items = vec![
            (sync(), ResponseState::Open),
            (approve(), ResponseState::Resurfaced),
            (claim(), ResponseState::Dismissed),
        ];
        assert_eq!(
            render_list(&items, false),
            [
                " 1. [1200] -  Sources are not synced",
                "    runs on its own | sync_stale | -:sync_stale:-",
                " 2. [690] PROJ-2  PROJ-2 passed testing: approve acme/web #7  (resurfaced)",
                "    asks first, external | approve_prs | PROJ-2:approve_prs:acme/web#7",
            ]
        );
    }

    #[test]
    fn all_shows_answered_suggestions_marked_and_no_limit() {
        let items = vec![
            (claim(), ResponseState::Dismissed),
            (approve(), ResponseState::Snoozed),
        ];
        let lines = render_list(&items, true);
        assert!(lines[0].ends_with("(dismissed)"));
        assert!(lines[2].ends_with("(snoozed)"));
        assert_eq!(render_list(&items, false), ["Nothing to do right now."]);
    }

    #[test]
    fn only_the_top_few_are_shown_without_all() {
        let items: Vec<_> = (0..8).map(|_| (claim(), ResponseState::Open)).collect();
        let lines = render_list(&items, false);
        assert_eq!(lines.len(), DEFAULT_LIMIT * 2 + 1);
        assert_eq!(lines.last().unwrap(), "    ... 3 more (de next --all)");
        assert_eq!(render_list(&items, true).len(), 16);
        assert_eq!(render_list(&[], false), ["Nothing to do right now."]);
    }

    #[test]
    fn the_json_shape_is_a_versioned_contract() {
        let items = vec![
            (claim(), ResponseState::Open),
            (approve(), ResponseState::Dismissed),
        ];
        let expected = r#"{
  "version": 1,
  "generated_at": 1700000000,
  "suggestions": [
    {
      "rank": 1,
      "id": "PROJ-1:claim_new:-",
      "ticket": "PROJ-1",
      "rule": "claim_new",
      "priority": 480,
      "level": "local_one_click",
      "informational": false,
      "state": "open",
      "action": {
        "type": "claim",
        "params": {
          "ticket": "PROJ-1"
        }
      },
      "reason": "PROJ-1 is in In Review and not claimed",
      "facts": {
        "ticket": "PROJ-1",
        "queue_position": 1
      }
    },
    {
      "rank": 2,
      "id": "PROJ-2:approve_prs:acme/web#7",
      "ticket": "PROJ-2",
      "rule": "approve_prs",
      "priority": 690,
      "level": "confirmed_external",
      "informational": false,
      "state": "dismissed",
      "action": {
        "type": "gateway",
        "params": {
          "approve_pr": {
            "repo": "acme/web",
            "pr": 7
          }
        }
      },
      "reason": "PROJ-2 passed testing: approve acme/web #7",
      "facts": {
        "repo": "acme/web",
        "pr": 7
      }
    }
  ]
}"#;
        assert_eq!(render_json(&items, 1_700_000_000), expected);
    }

    #[test]
    fn the_json_of_a_sync_and_an_empty_list() {
        let parsed: Value = serde_json::from_str(&render_json(&[], 5)).unwrap();
        assert_eq!(
            parsed,
            json!({ "version": 1, "generated_at": 5, "suggestions": [] })
        );

        let one = [(sync(), ResponseState::Open)];
        let parsed: Value = serde_json::from_str(&render_json(&one, 5)).unwrap();
        assert_eq!(
            parsed["suggestions"][0]["action"],
            json!({ "type": "sync" })
        );
        assert_eq!(parsed["suggestions"][0]["ticket"], Value::Null);
        assert_eq!(parsed["suggestions"][0]["level"], "automatic");
    }

    #[test]
    fn every_action_kind_serialises_with_its_type_name() {
        let t = key("PROJ-1");
        let actions = [
            SuggestedAction::Sync,
            SuggestedAction::Claim { ticket: t.clone() },
            SuggestedAction::StartReview { ticket: t.clone() },
            SuggestedAction::MarkReviewed { ticket: t.clone() },
            SuggestedAction::Activate {
                ticket: t.clone(),
                baseline: Some(BaselineChoice::Uat),
            },
            SuggestedAction::ChooseHotfixBaseline { ticket: t.clone() },
            SuggestedAction::Park { ticket: t.clone() },
            SuggestedAction::MarkDone { ticket: t.clone() },
            SuggestedAction::RunIntegrationPrepare { ticket: t.clone() },
            SuggestedAction::ComposeDeployComment {
                ticket: t.clone(),
                partial: true,
            },
            SuggestedAction::RevertOverlay {
                ticket: t.clone(),
                repos: vec!["web".into()],
            },
            SuggestedAction::ResolveConflict {
                ticket: t.clone(),
                repo: "web".into(),
                files: vec![],
            },
            SuggestedAction::OpenPr {
                repo: "r".into(),
                pr: 1,
                url: "u".into(),
            },
            SuggestedAction::OpenPipeline {
                repo: "r".into(),
                run_id: "x".into(),
                url: "u".into(),
            },
            SuggestedAction::OpenTicket {
                ticket: t.clone(),
                url: None,
            },
            SuggestedAction::Waiting {
                ticket: t.clone(),
                what: "w".into(),
            },
            SuggestedAction::AdapterUnavailable {
                adapter: "jira".into(),
                detail: "d".into(),
                blocked: "b".into(),
            },
            SuggestedAction::Gateway(Action::ApprovePr {
                repo: "r".into(),
                pr: 1,
            }),
        ];
        for a in actions {
            let v = serde_json::to_value(&a).unwrap();
            assert_eq!(v["type"], a.kind());
            assert!(!describe_action(&a).is_empty());
        }
    }

    #[test]
    fn describing_an_external_action_is_the_gateways_own_preview() {
        let lines = describe_action(&approve().action);
        assert!(lines[0].contains("acme/web") || lines.iter().any(|l| l.contains("#7")));
    }

    #[test]
    fn describing_a_hotfix_baseline_and_activation() {
        let a = SuggestedAction::Activate {
            ticket: key("PROJ-1"),
            baseline: Some(BaselineChoice::Production),
        };
        assert!(describe_action(&a)[0].contains("production baseline"));
        let a = SuggestedAction::Activate {
            ticket: key("PROJ-1"),
            baseline: None,
        };
        assert!(describe_action(&a)[0].contains("base baseline"));
    }
}

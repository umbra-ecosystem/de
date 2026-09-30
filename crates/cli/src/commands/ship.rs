//! `de ticket integrate | integration | deploy | draft-comment | post-comment | transition |
//! cancel-integration`: the `uat` integration and announce flow.
//!
//! Every external write goes through the core write gateway and is previewed and confirmed
//! by a person first (`dialoguer::Confirm`, default no). There is no flag that skips it;
//! without a terminal the commands refuse rather than assume a yes.

use std::io::IsTerminal;

use de_core::{
    activation::WorkspaceRepo,
    config::Config,
    domain::{LocalStatus, TicketKey},
    gateway::{Action, ActionPreview, Gateway, PushReport, PushResult},
    git::short_sha,
    integration::{
        ComposeOptions, DeployState, IntegrationPrep, RepoDeploy, RepoOutcome, cancel_integration,
        compose_deploy_comment, deploy_status, execute_push, finalize_integration, integration_dir,
        post_comment, post_transition, prepare_integration, preview_post_comment,
        preview_transition,
    },
    overlay::ProcessRunner,
    store::{
        Kind, Store,
        drafts::{self, DraftKind, DraftStatus},
        overlays,
        restore::{self, RepoRole},
        tickets, uat_details,
    },
    sync::HostedRepo,
    utils::get_project_dirs,
};
use dialoguer::Confirm;
use eyre::{Context, bail, eyre};

use super::ticket::render_deactivation;
use crate::{
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
};

fn now() -> eyre::Result<i64> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .wrap_err("The system clock is before 1970")?;
    Ok(since_epoch.as_secs() as i64)
}

fn data_dir() -> eyre::Result<std::path::PathBuf> {
    Ok(get_project_dirs()?.data_dir().to_path_buf())
}

fn workspace_repos(workspace: Option<Slug>) -> eyre::Result<Vec<WorkspaceRepo>> {
    let workspace = get_workspace_for_cli(Some(workspace))?;
    WorkspaceRepo::from_workspace(&workspace)
}

fn require_tracked(state: &Store, key: &TicketKey) -> eyre::Result<tickets::TicketTracking> {
    tickets::get(state, key)?
        .ok_or_else(|| eyre!("{key} is not tracked; start with `de ticket claim {key}`"))
}

/// Asks for a yes, defaulting to no. Refuses without a terminal: a confirmation that cannot
/// be asked is a no.
pub(crate) fn confirm(prompt: &str) -> eyre::Result<bool> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("This needs an interactive confirmation and there is no terminal; nothing was sent");
    }
    Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .wrap_err("Could not ask for confirmation; nothing was sent")
}

// ------------------------------------------------------------ pure: render

fn short(sha: &str) -> &str {
    short_sha(sha)
}

/// One block per touched repo: what preparing found.
pub fn render_prep(prep: &IntegrationPrep) -> Vec<String> {
    let mut lines = Vec::new();
    for r in &prep.repos {
        lines.push(format!(
            "{}  {} -> {}/{}",
            r.repo, r.ticket_branch, r.remote, r.uat_branch
        ));
        match &r.outcome {
            RepoOutcome::Ready(m) => lines.push(format!(
                "    ready: merge {} onto uat {} ({} commit(s), {} file(s))",
                short(&m.merge_commit),
                short(&m.uat_before),
                m.commits.len(),
                m.files.len()
            )),
            RepoOutcome::UpToDate { uat } => lines.push(format!(
                "    already in uat ({}); nothing to push",
                short(uat)
            )),
            RepoOutcome::AlreadyPushed { merge_commit } => lines.push(format!(
                "    already pushed earlier as {}; skipped",
                short(merge_commit)
            )),
            RepoOutcome::Conflict { files } => {
                lines.push("    CONFLICT with uat; not merged. Conflicting files:".into());
                lines.extend(files.iter().map(|f| format!("      {f}")));
            }
            RepoOutcome::Blocked { reason } => {
                let mut it = reason.lines();
                lines.push(format!("    BLOCKED: {}", it.next().unwrap_or("")));
                lines.extend(it.map(|l| format!("      {l}")));
            }
        }
        lines.extend(r.warnings.iter().map(|w| format!("    ! {w}")));
    }
    lines
}

/// What happened to each repo's push.
pub fn render_push_report(report: &PushReport) -> Vec<String> {
    report
        .repos
        .iter()
        .map(|r| match &r.result {
            PushResult::Pushed { merge_commit } => {
                format!("{}: pushed {} to uat and recorded it", r.repo, short(merge_commit))
            }
            PushResult::AlreadyPushed => format!("{}: already pushed; skipped", r.repo),
            PushResult::Rejected { reason } => format!(
                "{}: REJECTED, not forced. uat moved since the merge was prepared; run `de ticket integrate` again. ({})",
                r.repo,
                reason.lines().next().unwrap_or("")
            ),
            PushResult::Blocked { reason } => format!("{}: BLOCKED before pushing: {reason}", r.repo),
            PushResult::Failed { error } => format!("{}: push FAILED: {error}", r.repo),
            PushResult::PushedUnrecorded { merge_commit, error } => format!(
                "{}: pushed {} but could not record it locally ({error}); run `de ticket integrate` again to record it",
                r.repo,
                short(merge_commit)
            ),
            PushResult::NotAttempted => format!("{}: not attempted", r.repo),
        })
        .collect()
}

pub fn render_preview(preview: &ActionPreview) -> Vec<String> {
    preview.to_lines()
}

/// The per-repo deploy table.
pub fn render_deploy(status: &[RepoDeploy]) -> Vec<String> {
    if status.is_empty() {
        return vec!["No repos are linked to this ticket and nothing was pushed.".into()];
    }
    let width = status.iter().map(|r| r.repo.len()).max().unwrap_or(0);
    let mut lines = Vec::new();
    for r in status {
        let mut line = format!("{:<width$}  {:<10}", r.repo, r.state.as_str());
        if let Some(commit) = &r.merge_commit {
            line.push_str(&format!("  uat {}", short(commit)));
        }
        if let Some(run) = &r.run {
            line.push_str(&format!(
                "  pipeline {} ({}) {}",
                run.number
                    .map_or_else(|| run.id.clone(), |n| format!("#{n}")),
                run.state,
                run.url
            ));
        }
        match r.state {
            DeployState::Untracked => {
                line.push_str("  (no [hosting]; cannot follow its pipelines)")
            }
            DeployState::Pending => line.push_str("  (waiting for a pipeline; try `de sync`)"),
            _ => {}
        }
        lines.push(line);
    }
    let deployed = status
        .iter()
        .filter(|r| r.state == DeployState::Deployed)
        .count();
    lines.push(format!("{deployed} of {} repo(s) deployed.", status.len()));
    lines
}

/// The state of an integration for `de ticket integration`.
pub fn render_integration_state(
    status: LocalStatus,
    touched: &[(String, String)],
    merges: &[(String, String, String)],
    worktrees: bool,
) -> Vec<String> {
    let mut lines = vec![format!("status: {status}")];
    if touched.is_empty() {
        lines.push("touched repos: none recorded (the ticket is not active)".into());
    }
    for (repo, branch) in touched {
        let pushed = merges.iter().rfind(|(r, _, _)| r == repo);
        lines.push(match pushed {
            Some((_, commit, kind)) => format!(
                "{repo}: {branch}  {} in uat as {}",
                if kind == "already_in_uat" {
                    "already"
                } else {
                    "pushed"
                },
                short(commit)
            ),
            None => format!("{repo}: {branch}  not pushed"),
        });
    }
    for (repo, commit, _) in merges {
        if !touched.iter().any(|(r, _)| r == repo) {
            lines.push(format!("{repo}: pushed as {}", short(commit)));
        }
    }
    if worktrees {
        lines.push(
            "temporary integration worktrees exist (`de ticket cancel-integration` removes them)"
                .into(),
        );
    }
    lines
}

fn print_lines(ui: &UserInterface, lines: &[String]) -> eyre::Result<()> {
    for line in lines {
        ui.writeln(line)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- integrate

/// `de ticket integrate <KEY> [--dry-run]`.
pub fn integrate(key: TicketKey, dry_run: bool, workspace: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    let config = Config::load()?;
    require_tracked(&state, &key)?;
    let repos = workspace_repos(workspace)?;
    let data = data_dir()?;
    let runner = ProcessRunner::new();

    ui.heading(&format!("Preparing {key} for uat (nothing is pushed yet)"))?;
    let prep = prepare_integration(&state, &runner, &data, &key, &repos, now()?)?;
    print_lines(&ui, &render_prep(&prep))?;

    let outcome = integrate_from_prep(&ui, &state, &config, &data, &runner, &repos, &prep, dry_run);
    // Whatever happened, the temporary worktrees do not outlive this command unless the
    // ticket was integrated (finalize removes them itself).
    let _ = cancel_integration(&state, &data, &key, &repos, now()?);
    outcome
}

#[allow(clippy::too_many_arguments)]
fn integrate_from_prep(
    ui: &UserInterface,
    state: &Store,
    config: &Config,
    data: &std::path::Path,
    runner: &ProcessRunner,
    repos: &[WorkspaceRepo],
    prep: &IntegrationPrep,
    dry_run: bool,
) -> eyre::Result<()> {
    let key = &prep.ticket;
    if !prep.is_pushable() {
        bail!(
            "Cannot integrate {key} yet: fix what is blocked or conflicting above and run this again"
        );
    }

    let gateway = Gateway::from_config(state, config);

    if !prep.is_ready() {
        // Nothing to push: every touched repo is already in uat.
        ui.info_item("Nothing to push: every touched repo is already in uat.")?;
        print_lines(ui, &render_remaining(state, key)?)?;
        if dry_run {
            ui.info_item("Dry run: nothing was changed.")?;
            return Ok(());
        }
        if !confirm(
            "Finish integrating (revert the test overlay, restore your branches, mark it integrated)?",
        )? {
            ui.info_item("Cancelled; nothing was changed.")?;
            return Ok(());
        }
        return finish(ui, state, runner, data, prep, repos);
    }

    let draft = gateway.draft_because(
        prep.push_action()?,
        serde_json::json!({ "reason": "ticket tested locally and integrated by the author" }),
    );
    ui.new_line()?;
    print_lines(ui, &render_preview(draft.preview()))?;

    if dry_run {
        ui.info_item("Dry run: nothing was pushed, and the temporary worktrees are removed.")?;
        return Ok(());
    }
    ui.new_line()?;
    if !confirm(&format!(
        "Push the merges above to uat? This DEPLOYS {key} to the alpha environment"
    ))? {
        ui.info_item("Cancelled; nothing was pushed.")?;
        return Ok(());
    }

    let (report, executed) = execute_push(&gateway, draft.confirm(), now()?)?;
    print_lines(ui, &render_push_report(&report))?;
    for warning in &executed.audit_warnings {
        ui.warning_item(warning, None)?;
    }
    if !report.all_done() {
        bail!(
            "Not every repo was pushed. What was pushed stays recorded and {key} stays active; \
             run `de ticket integrate {key}` again to continue with the rest"
        );
    }
    finish(ui, state, runner, data, prep, repos)
}

/// What finishing an integration still has to do: the repos to put back on their branches
/// and the overlays to revert (pure over what is recorded).
fn render_remaining(state: &Store, key: &TicketKey) -> eyre::Result<Vec<String>> {
    let repos: Vec<String> = restore::list(state, key)?
        .into_iter()
        .map(|r| r.repo)
        .collect();
    let overlays: Vec<String> = overlays::list(state, key)?
        .into_iter()
        .map(|o| o.repo)
        .collect();
    let mut lines = Vec::new();
    if !repos.is_empty() {
        lines.push(format!(
            "Still to restore to their branches: {}",
            repos.join(", ")
        ));
    }
    if !overlays.is_empty() {
        lines.push(format!(
            "Test overlays still to revert: {}",
            overlays.join(", ")
        ));
    }
    Ok(lines)
}

fn finish(
    ui: &UserInterface,
    state: &Store,
    runner: &ProcessRunner,
    data: &std::path::Path,
    prep: &IntegrationPrep,
    repos: &[WorkspaceRepo],
) -> eyre::Result<()> {
    let report = finalize_integration(state, runner, data, prep, repos, now()?)?;
    ui.heading(&format!("Integrated {}", prep.ticket))?;
    print_lines(ui, &render_deactivation(&report.deactivation))?;
    if !report.deactivation.is_complete() {
        bail!(
            "{} is pushed and recorded but still active because some repos could not be restored; fix them and run `de ticket integrate {}` again (it will not push again). \
             Do not use `de ticket deactivate`: that parks the ticket instead of marking it integrated",
            prep.ticket,
            prep.ticket
        );
    }
    ui.success_item(
        &format!("{} is integrated", prep.ticket),
        Some(&format!(
            "Next: de sync, then de ticket deploy {}",
            prep.ticket
        )),
    )?;
    Ok(())
}

// ------------------------------------------------------- integration, deploy

/// `de ticket integration <KEY>`.
pub fn integration(key: TicketKey) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    let tracking = require_tracked(&state, &key)?;
    let touched: Vec<(String, String)> = restore::list(&state, &key)?
        .into_iter()
        .filter(|r| r.role == RepoRole::Ticket)
        .map(|r| (r.repo, r.branch))
        .collect();
    let merges: Vec<(String, String, String)> = uat_details::list_for_ticket(&state, &key)?
        .into_iter()
        .map(|m| {
            (
                m.merge.repo,
                m.merge.commit,
                m.details.map_or("merge", |d| d.kind.as_str()).to_string(),
            )
        })
        .collect();
    let worktrees = integration_dir(&data_dir()?, &key).exists();
    print_lines(
        &ui,
        &render_integration_state(tracking.status, &touched, &merges, worktrees),
    )
}

fn hosted_repos(workspace: Option<Slug>, config: &Config) -> Vec<HostedRepo> {
    workspace_repos(workspace)
        .map(|repos| HostedRepo::from_workspace_repos(&repos, config))
        .unwrap_or_default()
}

/// `de ticket deploy <KEY>`.
pub fn deploy(key: TicketKey, workspace: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    let cache = Store::open_default(Kind::Cache)?;
    let config = Config::load()?;
    require_tracked(&state, &key)?;
    let status = deploy_status(&state, &cache, &key, &hosted_repos(workspace, &config))?;
    print_lines(&ui, &render_deploy(&status))
}

// ------------------------------------------------------------ comment drafts

/// `de ticket draft-comment <KEY> [--partial] [--note ...]`.
pub fn draft_comment(
    key: TicketKey,
    partial: bool,
    note: Option<String>,
    workspace: Option<Slug>,
) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    let cache = Store::open_default(Kind::Cache)?;
    let config = Config::load()?;
    require_tracked(&state, &key)?;
    let draft = compose_deploy_comment(
        &state,
        &cache,
        &key,
        &hosted_repos(workspace, &config),
        ComposeOptions {
            partial,
            note: note.as_deref(),
        },
        now()?,
    )?;
    ui.heading(&format!("Draft {} for {key} (saved, not posted)", draft.id))?;
    for line in draft.body.lines() {
        ui.writeln(&format!("  | {line}"))?;
    }
    ui.info_item(&format!(
        "Post it with `de ticket post-comment {key}` (you will see it again and confirm)"
    ))?;
    Ok(())
}

fn open_comment_draft(state: &Store, key: &TicketKey) -> eyre::Result<drafts::StoredDraft> {
    drafts::latest(state, key, DraftKind::DeployComment, DraftStatus::Draft)?.ok_or_else(|| {
        eyre!(
            "{key} has no unposted deploy comment; create one with `de ticket draft-comment {key}`"
        )
    })
}

/// `de ticket post-comment <KEY>`.
pub fn post_comment_cmd(key: TicketKey) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    let config = Config::load()?;
    require_tracked(&state, &key)?;
    let draft = open_comment_draft(&state, &key)?;
    let gateway = Gateway::from_config(&state, &config);

    let preview = preview_post_comment(&gateway, &state, draft.id)?;
    print_lines(&ui, &render_preview(preview.preview()))?;
    ui.new_line()?;
    if !confirm(&format!("Post this comment on {key} in Jira?"))? {
        ui.info_item("Cancelled; nothing was posted. The draft is kept.")?;
        return Ok(());
    }
    let comment = post_comment(&gateway, &state, draft.id, preview.confirm(), now()?)?;
    ui.success_item(
        &format!("Posted comment {} on {key}", comment.id),
        Some(&format!("Next: de ticket transition {key}")),
    )?;
    Ok(())
}

/// `de ticket transition <KEY>`.
pub fn transition(key: TicketKey) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    let config = Config::load()?;
    require_tracked(&state, &key)?;
    let gateway = Gateway::from_config(&state, &config);

    let (id, preview) = preview_transition(&gateway, &state, &config, &key, now()?)?;
    print_lines(&ui, &render_preview(preview.preview()))?;
    ui.new_line()?;
    let Action::TransitionJira { to_status, .. } = preview.action().clone() else {
        bail!("unexpected action");
    };
    if !confirm(&format!("Move {key} to '{to_status}' in Jira?"))? {
        drafts::discard(&state, id)?;
        ui.info_item("Cancelled; nothing was changed.")?;
        return Ok(());
    }
    post_transition(&gateway, &state, id, preview.confirm(), now()?)?;
    ui.success_item(&format!("{key} moved to '{to_status}'"), None)?;
    Ok(())
}

/// `de ticket cancel-integration <KEY>`.
pub fn cancel(key: TicketKey, workspace: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = Store::open_default(Kind::State)?;
    require_tracked(&state, &key)?;
    let repos = workspace_repos(workspace)?;
    let removed = cancel_integration(&state, &data_dir()?, &key, &repos, now()?)?;
    if removed.is_empty() {
        ui.info_item("No temporary integration worktrees existed.")?;
    } else {
        ui.success_item(
            &format!(
                "Removed the integration worktrees of {}",
                removed.join(", ")
            ),
            None,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use de_core::{
        gateway::{PushReport, RepoPush},
        integration::ReadyMerge,
        integration::RepoIntegration,
        providers::{PipelineRun, PipelineState},
    };

    use super::*;

    fn integration_repo(outcome: RepoOutcome) -> RepoIntegration {
        RepoIntegration {
            repo: "web".into(),
            repo_dir: "/w".into(),
            remote: "origin".into(),
            uat_branch: "uat".into(),
            ticket_branch: "feature/PROJ-1-x".into(),
            ticket_tip: Some("a".repeat(40)),
            worktree: None,
            overlay_packages: vec![],
            outcome,
            warnings: vec!["web has uncommitted changes".into()],
        }
    }

    fn prep(outcomes: Vec<RepoOutcome>) -> IntegrationPrep {
        IntegrationPrep {
            ticket: "PROJ-1".parse().unwrap(),
            repos: outcomes.into_iter().map(integration_repo).collect(),
        }
    }

    #[test]
    fn prep_lines_show_each_outcome() {
        let lines = render_prep(&prep(vec![
            RepoOutcome::Ready(ReadyMerge {
                merge_commit: "c".repeat(40),
                uat_before: "b".repeat(40),
                commits: vec![],
                files: vec!["a".into(), "b".into()],
            }),
            RepoOutcome::Conflict {
                files: vec!["src/A.php".into()],
            },
            RepoOutcome::Blocked {
                reason: "line one\nline two".into(),
            },
            RepoOutcome::UpToDate {
                uat: "d".repeat(40),
            },
        ]))
        .join("\n");
        assert!(lines.contains("web  feature/PROJ-1-x -> origin/uat"));
        assert!(lines.contains("ready: merge cccccccc onto uat bbbbbbbb (0 commit(s), 2 file(s))"));
        assert!(lines.contains("CONFLICT") && lines.contains("      src/A.php"));
        assert!(lines.contains("BLOCKED: line one\n      line two"));
        assert!(lines.contains("already in uat (dddddddd)"));
        assert!(lines.contains("! web has uncommitted changes"));
    }

    #[test]
    fn push_reports_say_what_to_do_next() {
        let report = PushReport {
            repos: vec![
                RepoPush {
                    repo: "api".into(),
                    result: PushResult::Pushed {
                        merge_commit: "c".repeat(40),
                    },
                },
                RepoPush {
                    repo: "web".into(),
                    result: PushResult::Rejected {
                        reason: "uat moved\nmore".into(),
                    },
                },
                RepoPush {
                    repo: "worker".into(),
                    result: PushResult::NotAttempted,
                },
            ],
        };
        let lines = render_push_report(&report);
        assert_eq!(lines[0], "api: pushed cccccccc to uat and recorded it");
        assert!(
            lines[1].starts_with("web: REJECTED, not forced") && lines[1].contains("integrate")
        );
        assert!(
            !lines[1].contains("more"),
            "only the first line of the reason"
        );
        assert_eq!(lines[2], "worker: not attempted");
    }

    fn run() -> PipelineRun {
        PipelineRun {
            repo: "acme/web".into(),
            id: "{1}".into(),
            number: Some(45),
            state: PipelineState::Succeeded,
            branch: "uat".into(),
            commit: "c".repeat(40),
            created_at: 1,
            completed_at: None,
            url: "https://bb/45".into(),
            steps: vec![],
        }
    }

    #[test]
    fn deploy_table_counts_and_explains() {
        let rows = vec![
            RepoDeploy {
                repo: "web".into(),
                hosting_repo: Some("acme/web".into()),
                environment: Some("alpha".into()),
                merge_commit: Some("c".repeat(40)),
                state: DeployState::Deployed,
                run: Some(run()),
            },
            RepoDeploy {
                repo: "api".into(),
                hosting_repo: Some("acme/api".into()),
                environment: None,
                merge_commit: Some("d".repeat(40)),
                state: DeployState::Pending,
                run: None,
            },
            RepoDeploy {
                repo: "worker".into(),
                hosting_repo: None,
                environment: None,
                merge_commit: None,
                state: DeployState::NotPushed,
                run: None,
            },
        ];
        let lines = render_deploy(&rows);
        assert!(
            lines[0].contains("deployed")
                && lines[0].contains("pipeline #45 (succeeded) https://bb/45")
        );
        assert!(lines[1].contains("waiting for a pipeline"));
        assert!(lines[2].contains("not pushed"));
        assert_eq!(lines[3], "1 of 3 repo(s) deployed.");
        assert!(render_deploy(&[])[0].contains("No repos"));
    }

    #[test]
    fn integration_state_lists_pushed_and_unpushed_repos() {
        let lines = render_integration_state(
            LocalStatus::Active,
            &[("web".into(), "b1".into()), ("api".into(), "b2".into())],
            &[("web".into(), "c".repeat(40), "merge".into())],
            true,
        );
        assert_eq!(lines[0], "status: active");
        assert_eq!(lines[1], "web: b1  pushed in uat as cccccccc");
        assert_eq!(lines[2], "api: b2  not pushed");
        assert!(lines[3].contains("cancel-integration"));
        let none = render_integration_state(LocalStatus::Parked, &[], &[], false);
        assert!(none[1].contains("not active"));
    }
}

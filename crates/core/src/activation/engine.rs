//! Activating, parking and deactivating a ticket.
//!
//! Activation switches repos one at a time and writes a restore record to `state.db` right
//! after each switch, before doing anything else in that repo. Whatever fails, the records are
//! what deactivation (and the rollback of a failed activation) works from, so none of it
//! depends on memory of the process that started it.

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Context, bail, eyre};
use serde_json::{Value, json};

use super::{
    WorkspaceRepo, actions,
    discovery::{discover_links, gather_plan_repos},
    plan::{ActivationPlan, RepoAction, RepoPlan, plan_activation},
};
use crate::{
    domain::{AuditOutcome, BaselineChoice, LocalStatus, TicketKey},
    git::{FastForward, GitRepo, OnDirty, StashRef, SwitchOutcome},
    overlay::{
        ApplyRequest, CommandRunner, ExternalCommand, RevertOutcome, apply, revert, run_checked,
    },
    store::{
        Store,
        audit::{self, NewAuditEntry},
        links, overlays,
        restore::{self, RestoreRecord},
        tickets, time,
    },
};

#[derive(Debug, Clone)]
pub struct ActivateOptions {
    /// `git fetch` every repo first, so branches pushed since the last fetch are seen and
    /// baselines can be fast-forwarded. A failed fetch is a warning (offline still works).
    pub fetch: bool,
    /// Where repos that do not touch the ticket go. Only ever non-`Base` for hotfixes.
    pub baseline: BaselineChoice,
    /// The workspace's `default_branch`, the base of repos that do not configure one.
    pub workspace_default_branch: Option<String>,
}

impl Default for ActivateOptions {
    fn default() -> Self {
        Self {
            fetch: false,
            baseline: BaselineChoice::Base,
            workspace_default_branch: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoActivation {
    pub repo: String,
    pub action: RepoAction,
    pub previous_branch: Option<String>,
    pub previous_commit: Option<String>,
    /// The stash made for what was in the working tree; popped again on deactivation.
    pub stash: Option<StashRef>,
    /// The repo was already on the target branch.
    pub already_there: bool,
    /// Baseline repos: what the fast-forward did.
    pub fast_forward: Option<FastForward>,
    /// Composer packages the overlay pointed at their provider's checkout.
    pub overlaid: Vec<String>,
    /// Labels of the tasks that ran here.
    pub tasks_run: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationReport {
    pub ticket: TicketKey,
    pub repos: Vec<RepoActivation>,
    /// Non-fatal things worth showing: unreadable repos, diverged baselines, failed fetches.
    pub warnings: Vec<String>,
}

/// One repo put back by a deactivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRestore {
    pub repo: String,
    /// The branch (or `detached <sha>`) the repo is on again.
    pub restored_to: String,
    /// Uncommitted work found on the ticket's branch, stashed so nothing is lost. It is not
    /// popped back: it belongs to the ticket, not to what the repo was doing before.
    pub leftover_stash: Option<StashRef>,
    /// The stash made when activating was popped back.
    pub stash_popped: bool,
    /// The stash made when activating could not be applied cleanly. It is kept in git's
    /// stash list under this label and the working tree holds the conflicts.
    pub stash_kept: Option<(StashRef, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreFailure {
    pub repo: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeactivationReport {
    pub ticket: TicketKey,
    /// The overlays that were reverted.
    pub overlays: Vec<(String, RevertOutcome)>,
    pub repos: Vec<RepoRestore>,
    /// Repos that could not be restored. Their restore records are kept, the ticket stays
    /// as it was, and deactivating again retries exactly these.
    pub failures: Vec<RestoreFailure>,
    /// The status the ticket was moved to; `None` while `failures` is not empty.
    pub status: Option<LocalStatus>,
}

impl DeactivationReport {
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

/// The label of the stash made in `repo` for `ticket`, so it can be traced back.
pub fn stash_label(ticket: &TicketKey, repo: &str) -> String {
    format!("de:{ticket}:{repo}")
}

fn leftover_label(ticket: &TicketKey, repo: &str) -> String {
    format!("{}:leftover", stash_label(ticket, repo))
}

fn record(
    store: &Store,
    now: i64,
    action: &str,
    ticket: &TicketKey,
    repo: Option<&str>,
    details: Value,
    outcome: AuditOutcome,
) -> eyre::Result<()> {
    audit::append(
        store,
        &NewAuditEntry {
            at: now,
            action: action.into(),
            ticket: Some(ticket.clone()),
            repo: repo.map(String::from),
            details,
            outcome,
        },
    )?;
    Ok(())
}

/// Like [`record`] for paths that must carry on whatever happens (rollback).
fn record_best_effort(
    store: &Store,
    now: i64,
    action: &str,
    ticket: &TicketKey,
    repo: Option<&str>,
    details: Value,
    outcome: AuditOutcome,
) {
    if let Err(e) = record(store, now, action, ticket, repo, details, outcome) {
        tracing::warn!("Failed to write the audit entry {action}: {e:#}");
    }
}

/// Everything that must hold before any repo is touched.
fn check_preconditions(store: &Store, ticket: &TicketKey) -> eyre::Result<()> {
    let tracking = tickets::get(store, ticket)?
        .ok_or_else(|| eyre!("{ticket} is not tracked; claim it first"))?;

    if tracking.status == LocalStatus::Active {
        bail!("{ticket} is already active");
    }
    if let Some(other) = tickets::active(store)? {
        bail!(
            "{} is already active; park or deactivate it before activating {ticket}",
            other.key
        );
    }
    if !tracking.status.can_transition_to(LocalStatus::Active) {
        bail!(
            "{ticket} cannot be activated from status {}",
            tracking.status
        );
    }
    if let Some(open) = time::open_entry(store)? {
        bail!("A timer is still running on {}; stop it first", open.ticket);
    }

    // A stale record means some repo is still on an earlier activation's branch or still
    // carries its overlay; starting another one on top would lose track of both.
    let mut stale: BTreeSet<String> = restore::list_all(store)?
        .into_iter()
        .map(|r| format!("{} in {}", r.ticket, r.repo))
        .collect();
    stale.extend(
        overlays::list_all(store)?
            .into_iter()
            .map(|o| format!("{} overlay in {}", o.ticket, o.repo)),
    );
    if !stale.is_empty() {
        bail!(
            "An earlier activation was not cleaned up ({}); finish it with `de ticket deactivate <KEY>` first",
            stale.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}

/// The commands a repo runs, resolved from its tasks before anything is touched.
#[derive(Default)]
struct RepoCommands {
    rebuild: Vec<ExternalCommand>,
    after: Vec<ExternalCommand>,
}

fn resolve_commands(
    repos: &[WorkspaceRepo],
    plan: &ActivationPlan,
) -> eyre::Result<BTreeMap<String, RepoCommands>> {
    let mut commands = BTreeMap::new();
    let mut missing = Vec::new();

    for repo_plan in &plan.repos {
        let Some(repo) = repos.iter().find(|r| r.name == repo_plan.name) else {
            continue;
        };
        let project = repo.project();
        let mut resolve = |task: &str| -> Option<ExternalCommand> {
            match project.resolve_task(task) {
                Ok(Some(resolved)) => Some(ExternalCommand::from_command(
                    &resolved.to_command(&[]),
                    &repo.dir,
                    format!("task {task}"),
                )),
                Ok(None) => {
                    missing.push(format!("{}: task '{task}' is not defined", repo.name));
                    None
                }
                Err(e) => {
                    missing.push(format!(
                        "{}: task '{task}' could not be resolved: {e:#}",
                        repo.name
                    ));
                    None
                }
            }
        };

        let rebuild = repo_plan
            .overlay
            .iter()
            .flat_map(|o| &o.rebuild)
            .filter_map(|t| resolve(t))
            .collect();
        let after = repo_plan.after.iter().filter_map(|t| resolve(t)).collect();
        commands.insert(repo.name.clone(), RepoCommands { rebuild, after });
    }

    if !missing.is_empty() {
        bail!(
            "Cannot activate; tasks named in de.toml are missing:\n  - {}",
            missing.join("\n  - ")
        );
    }
    Ok(commands)
}

/// What to check out.
#[derive(Debug, Clone, Copy)]
enum Target<'a> {
    Branch(&'a str),
    Detached(&'a str),
}

/// Switches to `target`, stashing anything in the working tree under `label` first. Unlike
/// `GitRepo::switch`, this also stashes when the repo is already on the target: activation
/// wants the whole stack on known code, so local changes are set aside either way.
fn switch_keeping_work(
    repo: &GitRepo,
    target: Target<'_>,
    label: &str,
) -> eyre::Result<SwitchOutcome> {
    let status = repo.status()?;
    let already = match target {
        Target::Branch(branch) => status.branch.as_deref() == Some(branch),
        Target::Detached(commit) => status.detached && status.head.as_deref() == Some(commit),
    };

    if already {
        let stash = if status.is_clean() {
            None
        } else {
            repo.stash_push(label)?
        };
        return Ok(SwitchOutcome {
            previous_branch: status.branch,
            previous_commit: status.head,
            stash,
            created_tracking_branch: false,
            already_on_branch: true,
        });
    }

    let on_dirty = OnDirty::StashLabelled(label.into());
    match target {
        Target::Branch(branch) => repo.switch(branch, on_dirty),
        Target::Detached(commit) => repo.switch_detached(commit, on_dirty),
    }
}

/// Activates `ticket`: switches every repo (the ones with a ticket branch to that branch, the
/// rest to a baseline), applies the composer overlay where a provider is on the ticket branch,
/// runs rebuild tasks, and makes the ticket `Active` with a running timer.
///
/// Nothing is touched unless the preconditions hold and the plan is valid: the ticket is
/// tracked, none is active, no earlier activation is left half-undone, branches are not
/// ambiguous, every baseline exists and every task resolves.
///
/// Once repos are being switched, any failure rolls back the ones already done (best effort;
/// what could not be rolled back is named in the error and stays recorded for
/// [`deactivate`]) and leaves the ticket as it was.
pub fn activate(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    repos: &[WorkspaceRepo],
    opts: &ActivateOptions,
    now: i64,
) -> eyre::Result<ActivationReport> {
    check_preconditions(store, ticket)?;

    let mut warnings = Vec::new();

    // Fetching changes no working tree, so it is safe to do before planning; doing it first is
    // what lets a branch pushed since the last fetch count as the ticket's.
    if opts.fetch {
        for repo in repos {
            let fetched = GitRepo::open(&repo.dir).and_then(|git| git.fetch(repo.remote()));
            if let Err(e) = fetched {
                warnings.push(format!(
                    "Could not fetch {} (using what is local): {e:#}",
                    repo.name
                ));
            }
        }
    }

    discover_links(store, ticket, repos, now)?;
    let (plan_repos, unreadable) =
        gather_plan_repos(repos, ticket, opts.workspace_default_branch.as_deref());
    warnings.extend(unreadable);
    let ticket_links = links::list(store, ticket)?;

    let plan = plan_activation(ticket, &plan_repos, &ticket_links, opts.baseline)
        .map_err(eyre::Report::new)?;
    warnings.extend(plan.warnings.iter().cloned());
    let commands = resolve_commands(repos, &plan)?;

    record(
        store,
        now,
        actions::ACTIVATION_START,
        ticket,
        None,
        json!({
            "baseline": plan.baseline.as_str(),
            "fetch": opts.fetch,
            "repos": plan.repos.iter().map(|r| json!({
                "repo": r.name,
                "action": r.action.role().as_str(),
                "branch": r.action.branch(),
                "overlay": r.overlay.is_some(),
            })).collect::<Vec<_>>(),
        }),
        AuditOutcome::Success,
    )?;

    let mut done = Vec::new();
    let result = run_plan(
        store,
        runner,
        ticket,
        &plan,
        &commands,
        now,
        &mut warnings,
        &mut done,
    )
    .and_then(|()| finish_activation(store, ticket, now));

    match result {
        Ok(()) => {
            record(
                store,
                now,
                actions::ACTIVATION_COMPLETE,
                ticket,
                None,
                json!({ "repos": done.iter().map(|r: &RepoActivation| &r.repo).collect::<Vec<_>>() }),
                AuditOutcome::Success,
            )?;
            Ok(ActivationReport {
                ticket: ticket.clone(),
                repos: done,
                warnings,
            })
        }
        Err(error) => {
            let undone = undo(store, runner, ticket, now);
            let (rolled_back, stuck) = match &undone {
                Ok(u) => (
                    u.repos.iter().map(|r| r.repo.clone()).collect::<Vec<_>>(),
                    u.failures
                        .iter()
                        .map(|f| format!("{}: {}", f.repo, f.error))
                        .collect::<Vec<_>>(),
                ),
                Err(e) => (Vec::new(), vec![format!("{e:#}")]),
            };
            record_best_effort(
                store,
                now,
                actions::ACTIVATION_FAILED,
                ticket,
                None,
                json!({
                    "error": format!("{error:#}"),
                    "rolled_back": rolled_back,
                    "not_rolled_back": stuck,
                }),
                AuditOutcome::Failure,
            );

            let mut message = format!("Activating {ticket} failed");
            if rolled_back.is_empty() {
                message.push_str("; nothing had been changed");
            } else {
                message.push_str(&format!("; rolled back {}", rolled_back.join(", ")));
            }
            if !stuck.is_empty() {
                message.push_str(&format!(
                    ". These could NOT be rolled back and are still recorded (run `de ticket deactivate {ticket}` \
                     after fixing them):\n  - {}",
                    stuck.join("\n  - ")
                ));
            }
            Err(error.wrap_err(message))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_plan(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    plan: &ActivationPlan,
    commands: &BTreeMap<String, RepoCommands>,
    now: i64,
    warnings: &mut Vec<String>,
    done: &mut Vec<RepoActivation>,
) -> eyre::Result<()> {
    let no_commands = RepoCommands::default();
    for (position, repo_plan) in plan.repos.iter().enumerate() {
        let cmds = commands.get(&repo_plan.name).unwrap_or(&no_commands);
        let activated = activate_repo(
            store, runner, ticket, position, repo_plan, cmds, now, warnings,
        )
        .wrap_err_with(|| format!("Failed to activate {} in {}", ticket, repo_plan.name))?;
        done.push(activated);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn activate_repo(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    position: usize,
    plan: &RepoPlan,
    cmds: &RepoCommands,
    now: i64,
    warnings: &mut Vec<String>,
) -> eyre::Result<RepoActivation> {
    let repo = GitRepo::open(&plan.dir)?;
    let branch = plan.action.branch();

    let switched = switch_keeping_work(
        &repo,
        Target::Branch(branch),
        &stash_label(ticket, &plan.name),
    )
    .wrap_err_with(|| format!("Failed to switch {} to '{branch}'", plan.name))?;

    // Persist before doing anything else in this repo: from here on it can be restored.
    restore::insert(
        store,
        &RestoreRecord {
            ticket: ticket.clone(),
            repo: plan.name.clone(),
            position: position as i64,
            repo_dir: plan.dir.clone(),
            role: plan.action.role(),
            branch: branch.into(),
            previous_branch: switched.previous_branch.clone(),
            previous_commit: switched.previous_commit.clone(),
            stash: switched.stash.clone(),
            created_at: now,
        },
    )?;
    record(
        store,
        now,
        actions::ACTIVATION_REPO_SWITCHED,
        ticket,
        Some(&plan.name),
        json!({
            "role": plan.action.role().as_str(),
            "branch": branch,
            "previous_branch": switched.previous_branch,
            "previous_commit": switched.previous_commit,
            "stash": switched.stash.as_ref().map(|s| json!({ "label": s.label, "commit": s.commit })),
            "already_there": switched.already_on_branch,
        }),
        AuditOutcome::Success,
    )?;

    let mut activation = RepoActivation {
        repo: plan.name.clone(),
        action: plan.action.clone(),
        previous_branch: switched.previous_branch,
        previous_commit: switched.previous_commit,
        stash: switched.stash,
        already_there: switched.already_on_branch,
        fast_forward: None,
        overlaid: Vec::new(),
        tasks_run: Vec::new(),
    };

    if matches!(plan.action, RepoAction::FallBackToBaseline(_)) {
        match repo.fast_forward(branch) {
            Ok(outcome) => {
                match &outcome {
                    FastForward::Diverged { ahead, behind } => warnings.push(format!(
                        "{}: {branch} has diverged from its upstream (ahead {ahead}, behind {behind}); left as it is",
                        plan.name
                    )),
                    FastForward::NoUpstream => warnings.push(format!(
                        "{}: {branch} has no upstream, so it was not brought up to date",
                        plan.name
                    )),
                    FastForward::UpToDate | FastForward::Advanced { .. } => {}
                }
                activation.fast_forward = Some(outcome);
            }
            Err(e) => warnings.push(format!(
                "{}: could not bring {branch} up to date: {e:#}",
                plan.name
            )),
        }
    }

    if let Some(overlay) = &plan.overlay {
        let request = ApplyRequest {
            ticket,
            repo: &plan.name,
            consumer_dir: &plan.dir,
            packages: &overlay.packages,
            rebuild: &cmds.rebuild,
        };
        let packages: Vec<&str> = overlay
            .packages
            .iter()
            .map(|p| p.package.as_str())
            .collect();

        match apply(store, runner, &request, now) {
            Ok(outcome) => {
                record(
                    store,
                    now,
                    actions::OVERLAY_APPLY,
                    ticket,
                    Some(&plan.name),
                    json!({
                        "packages": packages,
                        "providers": overlay.packages.iter().map(|p| &p.provider).collect::<Vec<_>>(),
                        "urls": outcome.urls,
                        "rebuilt": outcome.rebuilt,
                    }),
                    AuditOutcome::Success,
                )?;
                activation.overlaid = packages.iter().map(|p| String::from(*p)).collect();
                activation.tasks_run.extend(outcome.rebuilt);
            }
            Err(e) => {
                record_best_effort(
                    store,
                    now,
                    actions::OVERLAY_APPLY,
                    ticket,
                    Some(&plan.name),
                    json!({ "packages": packages, "error": format!("{e:#}") }),
                    AuditOutcome::Failure,
                );
                return Err(e.wrap_err(format!("Failed to apply the overlay in {}", plan.name)));
            }
        }
    }

    for task in &cmds.after {
        run_checked(runner, task)?;
        activation.tasks_run.push(task.label.clone());
    }

    Ok(activation)
}

/// The last step of an activation: the ticket becomes `Active` and its timer starts, together
/// or not at all.
fn finish_activation(store: &Store, ticket: &TicketKey, now: i64) -> eyre::Result<()> {
    let tx = store.conn().unchecked_transaction()?;
    tickets::set_status(store, ticket, LocalStatus::Active, now)?;
    time::start(store, ticket, now)?;
    tx.commit().wrap_err("Failed to record the activation")
}

#[derive(Default)]
struct Undone {
    overlays: Vec<(String, RevertOutcome)>,
    repos: Vec<RepoRestore>,
    failures: Vec<RestoreFailure>,
}

/// Undoes what is recorded for `ticket`: reverts overlays first, then puts every repo back on
/// its previous branch (stashing what testing left in the tree), then pops the original
/// stash. Works purely from `state.db`. A repo is forgotten only once it is fully restored.
fn undo(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    now: i64,
) -> eyre::Result<Undone> {
    let mut undone = Undone::default();
    let mut blocked: BTreeSet<String> = BTreeSet::new();

    for backup in overlays::list(store, ticket)? {
        match revert(store, runner, ticket, &backup.repo) {
            Ok(outcome) => {
                record_best_effort(
                    store,
                    now,
                    actions::OVERLAY_REVERT,
                    ticket,
                    Some(&backup.repo),
                    json!({ "outcome": format!("{outcome:?}") }),
                    AuditOutcome::Success,
                );
                undone.overlays.push((backup.repo, outcome));
            }
            Err(e) => {
                record_best_effort(
                    store,
                    now,
                    actions::OVERLAY_REVERT,
                    ticket,
                    Some(&backup.repo),
                    json!({ "error": format!("{e:#}") }),
                    AuditOutcome::Failure,
                );
                // Leave the branch alone while the overlay is still in place.
                blocked.insert(backup.repo.clone());
                undone.failures.push(RestoreFailure {
                    repo: backup.repo,
                    error: format!("{e:#}"),
                });
            }
        }
    }

    let mut records = restore::list(store, ticket)?;
    records.reverse();
    for rec in records {
        if blocked.contains(&rec.repo) {
            continue;
        }
        match restore_repo(store, ticket, &rec, now) {
            Ok(restored) => undone.repos.push(restored),
            Err(e) => {
                record_best_effort(
                    store,
                    now,
                    actions::DEACTIVATION_REPO_RESTORED,
                    ticket,
                    Some(&rec.repo),
                    json!({ "error": format!("{e:#}") }),
                    AuditOutcome::Failure,
                );
                undone.failures.push(RestoreFailure {
                    repo: rec.repo,
                    error: format!("{e:#}"),
                });
            }
        }
    }

    Ok(undone)
}

/// Puts one repo back and clears its record.
fn restore_repo(
    store: &Store,
    ticket: &TicketKey,
    rec: &RestoreRecord,
    now: i64,
) -> eyre::Result<RepoRestore> {
    let repo = GitRepo::open(&rec.repo_dir)?;
    let target = match (&rec.previous_branch, &rec.previous_commit) {
        (Some(branch), _) => Target::Branch(branch),
        (None, Some(commit)) => Target::Detached(commit),
        (None, None) => bail!("{} has no recorded previous state to return to", rec.repo),
    };

    // Whatever testing left behind is set aside, never discarded.
    let leftover = switch_keeping_work(&repo, target, &leftover_label(ticket, &rec.repo))
        .wrap_err_with(|| format!("Failed to switch {} back", rec.repo))?
        .stash;

    let mut restored = RepoRestore {
        repo: rec.repo.clone(),
        restored_to: match target {
            Target::Branch(branch) => branch.into(),
            Target::Detached(commit) => format!("detached {}", crate::git::short_sha(commit)),
        },
        leftover_stash: leftover,
        stash_popped: false,
        stash_kept: None,
    };

    if let Some(stash) = &rec.stash {
        let present = repo.stash_list()?.iter().any(|e| {
            e.commit == stash.commit || e.message.ends_with(&format!(": {}", stash.label))
        });
        // Already gone means an earlier attempt popped it and stopped before clearing the record.
        if present {
            match repo.stash_pop(stash) {
                Ok(()) => restored.stash_popped = true,
                Err(e) => restored.stash_kept = Some((stash.clone(), format!("{e:#}"))),
            }
        }
    }

    restore::delete(store, ticket, &rec.repo)?;
    record(
        store,
        now,
        actions::DEACTIVATION_REPO_RESTORED,
        ticket,
        Some(&rec.repo),
        json!({
            "restored_to": restored.restored_to,
            "stash_popped": restored.stash_popped,
            "stash_kept": restored.stash_kept.as_ref().map(|(s, why)| json!({ "label": s.label, "why": why })),
            "leftover_stash": restored.leftover_stash.as_ref().map(|s| &s.label),
        }),
        if restored.stash_kept.is_some() {
            AuditOutcome::Failure
        } else {
            AuditOutcome::Success
        },
    )?;
    Ok(restored)
}

/// Deactivates `ticket`: reverts overlays, restores every repo, stops the timer and moves the
/// ticket to `target` (validated against the status transition table before anything is done).
///
/// If some repo cannot be restored the report says which (`failures`), the ticket stays `Active`
/// with its timer running, and calling this again retries only what is left. Also finishes
/// the job for a ticket that is no longer `Active` but still has records (a rollback that
/// could not complete); its status is then left alone.
pub fn deactivate(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    target: LocalStatus,
    now: i64,
) -> eyre::Result<DeactivationReport> {
    let tracking = tickets::get(store, ticket)?.ok_or_else(|| eyre!("{ticket} is not tracked"))?;
    let active = tracking.status == LocalStatus::Active;
    let pending =
        !restore::list(store, ticket)?.is_empty() || !overlays::list(store, ticket)?.is_empty();

    if !active && !pending {
        bail!("{ticket} is not active");
    }
    if active && !LocalStatus::Active.can_transition_to(target) {
        bail!("{ticket} cannot go from active to {target}");
    }

    let undone = undo(store, runner, ticket, now)?;
    let mut report = DeactivationReport {
        ticket: ticket.clone(),
        overlays: undone.overlays,
        repos: undone.repos,
        failures: undone.failures,
        status: None,
    };

    if !report.is_complete() {
        record(
            store,
            now,
            actions::DEACTIVATION_INCOMPLETE,
            ticket,
            None,
            json!({ "failed": report.failures.iter().map(|f| &f.repo).collect::<Vec<_>>() }),
            AuditOutcome::Failure,
        )?;
        return Ok(report);
    }

    let tx = store.conn().unchecked_transaction()?;
    if time::open_entry(store)?.is_some_and(|open| &open.ticket == ticket) {
        time::stop(store, now)?;
    }
    if active {
        tickets::set_status(store, ticket, target, now)?;
        report.status = Some(target);
    }
    record(
        store,
        now,
        actions::DEACTIVATION_COMPLETE,
        ticket,
        None,
        json!({
            "status": active.then(|| target.as_str()),
            "repos": report.repos.iter().map(|r| &r.repo).collect::<Vec<_>>(),
        }),
        AuditOutcome::Success,
    )?;
    tx.commit().wrap_err("Failed to record the deactivation")?;

    Ok(report)
}

/// Sets the ticket aside with its notes and checklist: [`deactivate`] to `Parked`.
pub fn park(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    now: i64,
) -> eyre::Result<DeactivationReport> {
    deactivate(store, runner, ticket, LocalStatus::Parked, now)
}

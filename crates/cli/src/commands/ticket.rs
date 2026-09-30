//! `de ticket ...`: a thin headless surface over the ticket store and the activation engine.
//!
//! Each command gathers data (impure), then renders it with the pure `render_*` functions,
//! which are the parts covered by unit tests.

use std::collections::BTreeMap;

use de_core::{
    activation::{
        ActivateOptions, ActivationReport, DeactivationReport, RepoAction, RepoMatches,
        WorkspaceRepo, activate, deactivate, find_matches,
    },
    domain::{BaselineChoice, LocalStatus, RepoLinkOrigin, TicketKey, TicketKind},
    overlay::{ProcessRunner, RevertOutcome},
    store::{
        Kind, Store,
        jira_cache::{self, JiraTicket},
        links::{self, RepoLink},
        notes::{self, ChecklistItem, Note},
        overlays, restore, tickets, time,
    },
};
use dialoguer::Select;
use eyre::{Context, bail, eyre};

use crate::{
    cli::CheckCommands,
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
    workspace::Workspace,
};

// ------------------------------------------------------------------ helpers

fn now() -> eyre::Result<i64> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .wrap_err("The system clock is before 1970")?;
    Ok(since_epoch.as_secs() as i64)
}

fn open_state() -> eyre::Result<Store> {
    Store::open_default(Kind::State)
}

fn require_tracked(state: &Store, key: &TicketKey) -> eyre::Result<tickets::TicketTracking> {
    tickets::get(state, key)?
        .ok_or_else(|| eyre!("{key} is not tracked; start with `de ticket claim {key}`"))
}

/// The cached title of a ticket, if a cache row exists.
fn cached_title(key: &TicketKey) -> Option<String> {
    let cache = Store::open_default(Kind::Cache).ok()?;
    jira_cache::get(&cache, key).ok().flatten().map(|t| t.title)
}

fn workspace_repos(workspace: Option<Slug>) -> eyre::Result<(Workspace, Vec<WorkspaceRepo>)> {
    let workspace = get_workspace_for_cli(Some(workspace))?;
    let repos = WorkspaceRepo::from_workspace(&workspace)?;
    Ok((workspace, repos))
}

// ------------------------------------------------------- pure: formatting

/// `1h 05m`, `12m`, or `<1m`.
pub fn format_duration(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "<1m".into(),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

/// `YYYY-MM-DD HH:MM` in UTC.
pub fn format_timestamp(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);

    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

// ------------------------------------------------------------------- claim

pub fn claim(key: TicketKey, title: Option<String>, hotfix: bool) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;
    let now = now()?;

    tickets::claim(&state, &key, now)?;
    if hotfix {
        tickets::set_kind_override(&state, &key, Some(TicketKind::Hotfix), now)?;
    }
    if let Some(title) = &title {
        let cache = Store::open_default(Kind::Cache)?;
        jira_cache::upsert(
            &cache,
            &JiraTicket {
                key: key.clone(),
                title: title.clone(),
                jira_status: "unknown".into(),
                priority: None,
                assignee: None,
                url: None,
                raw_json: "{}".into(),
                fetched_at: now,
            },
        )?;
    }

    ui.success_item(
        &format!(
            "Claimed {key}{}{}",
            title.map(|t| format!(" \"{t}\"")).unwrap_or_default(),
            if hotfix { " (hotfix)" } else { "" }
        ),
        Some(&format!("Next: de ticket activate {key}")),
    )?;
    Ok(())
}

// -------------------------------------------------------------------- list

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub key: String,
    pub status: LocalStatus,
    pub kind: TicketKind,
    pub seconds: i64,
    pub title: Option<String>,
}

pub fn render_list(rows: &[ListRow]) -> Vec<String> {
    if rows.is_empty() {
        return vec!["No tickets are tracked. Start with `de ticket claim <KEY>`.".into()];
    }

    let key_width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0).max(3);
    let status_width = rows
        .iter()
        .map(|r| r.status.as_str().len())
        .max()
        .unwrap_or(0)
        .max(6);

    let mut lines = vec![format!(
        "{:<key_width$}  {:<status_width$}  {:<7}  {:>8}  TITLE",
        "KEY", "STATUS", "KIND", "TIME"
    )];
    for row in rows {
        lines.push(
            format!(
                "{:<key_width$}  {:<status_width$}  {:<7}  {:>8}  {}",
                row.key,
                row.status.as_str(),
                row.kind.as_str(),
                format_duration(row.seconds),
                row.title.as_deref().unwrap_or("")
            )
            .trim_end()
            .into(),
        );
    }
    lines
}

pub fn list() -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;
    let now = now()?;

    let mut rows = Vec::new();
    for t in tickets::list(&state)? {
        rows.push(ListRow {
            title: cached_title(&t.key),
            seconds: time::total_seconds(&state, &t.key, now)?,
            kind: t.kind_override.unwrap_or(TicketKind::Normal),
            status: t.status,
            key: t.key.to_string(),
        });
    }

    for (i, line) in render_list(&rows).iter().enumerate() {
        if i == 0 && !rows.is_empty() {
            ui.subheading(line)?;
        } else {
            ui.writeln(line)?;
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- show

/// One repo of a ticket as `show` displays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLine {
    pub repo: String,
    /// `None`: found by matching the key but not recorded as a link yet.
    pub origin: Option<RepoLinkOrigin>,
    /// The branches in play: the manual choice, or every branch matching the key.
    pub branches: Vec<String>,
}

/// Combine the stored links with what matching the key finds right now (when a workspace is
/// available). A manual branch wins over discovery; an excluded repo stays listed but hidden.
pub fn merge_repo_lines(links: &[RepoLink], live: Option<&[RepoMatches]>) -> Vec<RepoLine> {
    let mut lines: BTreeMap<String, RepoLine> = BTreeMap::new();

    for link in links {
        lines.insert(
            link.repo.clone(),
            RepoLine {
                repo: link.repo.clone(),
                origin: Some(link.origin),
                branches: link.branch.iter().cloned().collect(),
            },
        );
    }

    for found in live.unwrap_or_default() {
        let line = lines.entry(found.repo.clone()).or_insert_with(|| RepoLine {
            repo: found.repo.clone(),
            origin: None,
            branches: Vec::new(),
        });
        let manual_choice =
            line.origin == Some(RepoLinkOrigin::Manual) && !line.branches.is_empty();
        if !manual_choice && line.origin != Some(RepoLinkOrigin::Excluded) {
            line.branches = found.branches.clone();
        }
    }

    lines.into_values().collect()
}

/// Everything `show` displays.
#[derive(Debug, Clone)]
pub struct ShowData {
    pub key: TicketKey,
    pub title: Option<String>,
    pub tracking: tickets::TicketTracking,
    pub kind: TicketKind,
    pub repos: Vec<RepoLine>,
    /// Repos where an overlay is currently applied.
    pub overlays: Vec<String>,
    pub checklist: Vec<ChecklistItem>,
    pub notes: Vec<Note>,
    pub seconds: i64,
    pub timer_running: bool,
    /// Whether the repo list reflects the current branches (a workspace was available).
    pub live: bool,
}

pub fn render_show(data: &ShowData) -> Vec<String> {
    let mut lines = vec![format!(
        "{}  [{}]{}{}",
        data.key,
        data.tracking.status,
        if data.kind == TicketKind::Hotfix {
            "  hotfix"
        } else {
            ""
        },
        data.title
            .as_ref()
            .map(|t| format!("  {t}"))
            .unwrap_or_default()
    )];
    lines.push(format!(
        "time: {}{}",
        format_duration(data.seconds),
        if data.timer_running { " (running)" } else { "" }
    ));

    lines.push(String::new());
    lines.push(if data.live {
        "Repos:".into()
    } else {
        "Repos (from stored links; no workspace to look at):".into()
    });
    if data.repos.is_empty() {
        lines.push("  (no repo has a branch for this ticket)".into());
    }
    let width = data.repos.iter().map(|r| r.repo.len()).max().unwrap_or(0);
    for repo in &data.repos {
        let origin = match repo.origin {
            Some(RepoLinkOrigin::Auto) => "auto",
            Some(RepoLinkOrigin::Manual) => "manual",
            Some(RepoLinkOrigin::Excluded) => "excluded",
            None => "found",
        };
        let branches = match repo.branches.as_slice() {
            [] => "-".to_string(),
            [only] => only.clone(),
            many => format!(
                "{}  <- several; choose with `de ticket link {} {} --branch <BRANCH>`",
                many.join(", "),
                data.key,
                repo.repo
            ),
        };
        lines.push(format!("  {:<width$}  {branches}  ({origin})", repo.repo));
    }
    for repo in &data.overlays {
        lines.push(format!("  test overlay applied in {repo}"));
    }

    lines.push(String::new());
    lines.push("Checklist:".into());
    lines.extend(
        render_checklist(&data.checklist)
            .into_iter()
            .map(|l| format!("  {l}")),
    );

    lines.push(String::new());
    lines.push("Notes:".into());
    if data.notes.is_empty() {
        lines.push("  (none)".into());
    }
    for note in &data.notes {
        lines.push(format!(
            "  {}  {}",
            format_timestamp(note.created_at),
            note.body
        ));
    }
    lines
}

pub fn show(key: TicketKey, workspace: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;
    let now = now()?;
    let tracking = require_tracked(&state, &key)?;

    // A missing or broken workspace only costs the live branch view.
    let live: Option<Vec<RepoMatches>> = workspace_repos(workspace)
        .ok()
        .map(|(_, repos)| find_matches(&key, &repos).matches);

    let data = ShowData {
        title: cached_title(&key),
        kind: tracking.kind_override.unwrap_or(TicketKind::Normal),
        repos: merge_repo_lines(&links::list(&state, &key)?, live.as_deref()),
        overlays: overlays::list(&state, &key)?
            .into_iter()
            .map(|o| o.repo)
            .collect(),
        checklist: notes::list_checklist(&state, &key)?,
        notes: notes::list_notes(&state, &key)?,
        seconds: time::total_seconds(&state, &key, now)?,
        timer_running: time::open_entry(&state)?.is_some_and(|e| e.ticket == key),
        live: live.is_some(),
        tracking,
        key,
    };

    for (i, line) in render_show(&data).iter().enumerate() {
        if i == 0 {
            ui.heading(line)?;
        } else {
            ui.writeln(line)?;
        }
    }
    Ok(())
}

// -------------------------------------------------------------------- link

/// What `de ticket link` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// Link by hand, with a branch or leaving the branch to discovery.
    Manual(Option<String>),
    Exclude,
}

pub fn link_action(branch: Option<String>, exclude: bool) -> eyre::Result<LinkAction> {
    match (branch, exclude) {
        (Some(_), true) => bail!("--branch and --exclude cannot be combined"),
        (branch, false) => Ok(LinkAction::Manual(branch)),
        (None, true) => Ok(LinkAction::Exclude),
    }
}

pub fn link(
    key: TicketKey,
    repo: String,
    branch: Option<String>,
    exclude: bool,
    workspace: Option<Slug>,
) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let action = link_action(branch, exclude)?;
    let state = open_state()?;
    require_tracked(&state, &key)?;

    let workspace = get_workspace_for_cli(Some(workspace))?;
    if !workspace
        .config()
        .projects
        .keys()
        .any(|p| p.as_str() == repo)
    {
        let known: Vec<&str> = workspace
            .config()
            .projects
            .keys()
            .map(Slug::as_str)
            .collect();
        bail!(
            "'{repo}' is not a project of workspace '{}' (known: {})",
            workspace.config().name,
            known.join(", ")
        );
    }

    match action {
        LinkAction::Manual(branch) => {
            links::add_manual(&state, &key, &repo, branch.as_deref())?;
            ui.success_item(
                &match &branch {
                    Some(b) => format!("{key} uses branch '{b}' in {repo}"),
                    None => format!("{repo} is linked to {key}"),
                },
                None,
            )?;
        }
        LinkAction::Exclude => {
            links::exclude(&state, &key, &repo)?;
            ui.success_item(&format!("{repo} is hidden from {key}"), None)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- activate

/// The baseline for untouched repos. Normal tickets always use the base branch; hotfixes use
/// the flag or, without one, whatever `ask` returns. A flag on a normal ticket is ignored with a
/// warning.
pub fn resolve_baseline(
    kind: TicketKind,
    flag: Option<BaselineChoice>,
    ask: impl FnOnce() -> eyre::Result<BaselineChoice>,
) -> eyre::Result<(BaselineChoice, Option<String>)> {
    match (kind, flag) {
        (TicketKind::Hotfix, Some(choice)) => Ok((choice, None)),
        (TicketKind::Hotfix, None) => Ok((ask()?, None)),
        (TicketKind::Normal, None | Some(BaselineChoice::Base)) => Ok((BaselineChoice::Base, None)),
        (TicketKind::Normal, Some(other)) => Ok((
            BaselineChoice::Base,
            Some(format!(
                "--baseline {other} only applies to hotfixes; using the base branch"
            )),
        )),
    }
}

pub(crate) fn ask_baseline() -> eyre::Result<BaselineChoice> {
    let choices = BaselineChoice::ALL;
    let labels = [
        "base       (develop, or the repo's configured base)",
        "production (master or main)",
        "uat",
    ];
    let picked = Select::new()
        .with_prompt("Hotfix: which branch should repos this ticket does not touch use?")
        .items(&labels)
        .default(0)
        .interact()
        .wrap_err("Could not ask for the baseline (pass --baseline base|production|uat)")?;
    Ok(choices[picked])
}

pub fn render_activation(report: &ActivationReport) -> Vec<String> {
    let mut lines = Vec::new();
    let width = report.repos.iter().map(|r| r.repo.len()).max().unwrap_or(0);

    for repo in &report.repos {
        let (arrow, role) = match &repo.action {
            RepoAction::SwitchToTicketBranch(b) => (b.as_str(), "ticket branch"),
            RepoAction::FallBackToBaseline(b) => (b.as_str(), "baseline"),
        };
        let was = match (&repo.previous_branch, &repo.previous_commit) {
            (Some(b), _) if repo.already_there => format!("already on {b}"),
            (Some(b), _) => format!("was {b}"),
            (None, Some(c)) => format!("was detached at {}", de_core::git::short_sha(c)),
            (None, None) => "was empty".into(),
        };
        lines.push(format!("{:<width$}  {arrow}  ({role}; {was})", repo.repo));
        if let Some(stash) = &repo.stash {
            lines.push(format!("    stashed local changes as '{}'", stash.label));
        }
        if let Some(ff) = &repo.fast_forward {
            use de_core::git::FastForward::*;
            let text = match ff {
                UpToDate => "up to date".to_string(),
                Advanced { from, to } => format!(
                    "fast-forwarded {}..{}",
                    de_core::git::short_sha(from),
                    de_core::git::short_sha(to)
                ),
                Diverged { ahead, behind } => {
                    format!("diverged (ahead {ahead}, behind {behind}); left as is")
                }
                NoUpstream => "no upstream".to_string(),
            };
            lines.push(format!("    {text}"));
        }
        for package in &repo.overlaid {
            lines.push(format!("    test overlay: {package} -> provider checkout"));
        }
        for task in &repo.tasks_run {
            lines.push(format!("    ran {task}"));
        }
    }
    lines
}

pub fn activate_cmd(
    key: TicketKey,
    baseline: Option<BaselineChoice>,
    fetch: bool,
    workspace: Option<Slug>,
) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;
    let tracking = require_tracked(&state, &key)?;

    let (baseline, warning) = resolve_baseline(
        tracking.kind_override.unwrap_or(TicketKind::Normal),
        baseline,
        ask_baseline,
    )?;
    if let Some(warning) = warning {
        ui.warning_item(&warning, None)?;
    }

    let (workspace, repos) = workspace_repos(workspace)?;
    let options = ActivateOptions {
        fetch,
        baseline,
        workspace_default_branch: workspace.config().default_branch.clone(),
    };
    let report = activate(
        &state,
        &ProcessRunner::new(),
        &key,
        &repos,
        &options,
        now()?,
    )?;

    ui.heading(&format!("Activated {key}"))?;
    for line in render_activation(&report) {
        ui.writeln(&line)?;
    }
    for warning in &report.warnings {
        ui.warning_item(warning, None)?;
    }
    Ok(())
}

// -------------------------------------------------------------- deactivate

pub fn render_deactivation(report: &DeactivationReport) -> Vec<String> {
    let mut lines = Vec::new();

    for (repo, outcome) in &report.overlays {
        lines.push(match outcome {
            RevertOutcome::Reverted {
                lock_removed: true, ..
            } => format!(
                "{repo}: test overlay reverted (composer.lock did not exist before; removed)"
            ),
            RevertOutcome::Reverted {
                lock_removed: false,
                ..
            } => {
                format!("{repo}: test overlay reverted, composer files restored")
            }
            RevertOutcome::NothingToRevert => format!("{repo}: no test overlay to revert"),
        });
        if let RevertOutcome::Reverted { saved, .. } = outcome {
            for name in saved {
                lines.push(format!(
                    "  ! {name} was edited while testing; your version is saved as {name}.de-edited (see {repo})"
                ));
            }
        }
    }
    for repo in &report.repos {
        let mut line = format!("{}: back on {}", repo.repo, repo.restored_to);
        if repo.stash_popped {
            line.push_str(", your stashed changes are restored");
        }
        lines.push(line);
        if let Some(leftover) = &repo.leftover_stash {
            lines.push(format!(
                "  ! changes made while testing were stashed as '{}'",
                leftover.label
            ));
        }
        if let Some((stash, why)) = &repo.stash_kept {
            lines.push(format!(
                "  ! your changes could not be applied and stay in stash '{}': {}",
                stash.label,
                why.lines().next().unwrap_or(why)
            ));
        }
    }
    for failure in &report.failures {
        lines.push(format!(
            "{}: NOT restored: {}",
            failure.repo,
            failure.error.lines().next().unwrap_or(&failure.error)
        ));
    }
    lines
}

fn print_deactivation(ui: &UserInterface, report: &DeactivationReport) -> eyre::Result<()> {
    ui.heading(&format!("Deactivated {}", report.ticket))?;
    for line in render_deactivation(report) {
        ui.writeln(&line)?;
    }

    if !report.is_complete() {
        bail!(
            "{} is still active because some repos could not be restored; fix them and run this again",
            report.ticket
        );
    }
    if let Some(status) = report.status {
        ui.success_item(&format!("{} is now {status}", report.ticket), None)?;
    }
    Ok(())
}

/// Which ticket to deactivate: the one named, else the active one, else the only one an
/// interrupted activation left restore points for (a crash or a rollback that could not finish).
pub fn pick_ticket_to_deactivate(
    explicit: Option<TicketKey>,
    active: Option<TicketKey>,
    with_leftovers: &[TicketKey],
) -> eyre::Result<TicketKey> {
    if let Some(key) = explicit.or(active) {
        return Ok(key);
    }
    match with_leftovers {
        [] => bail!("No ticket is active"),
        [only] => Ok(only.clone()),
        many => bail!(
            "An earlier activation was left unfinished for several tickets ({}); name one",
            many.iter()
                .map(TicketKey::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The status a deactivated ticket may be moved to by hand.
///
/// `Integrated` is refused: it means "merged and pushed to `uat`", so only the flow that
/// actually pushes may set it. Marking it here would record a push that never happened.
fn manual_deactivation_target(status: Option<LocalStatus>) -> eyre::Result<LocalStatus> {
    match status.unwrap_or(LocalStatus::Parked) {
        LocalStatus::Integrated => Err(eyre::eyre!(
            "A ticket becomes 'integrated' only when it is merged and pushed to uat, not by hand"
        )),
        other => Ok(other),
    }
}

/// `de ticket deactivate` and `de ticket park`.
pub fn deactivate_cmd(key: Option<TicketKey>, status: Option<LocalStatus>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;

    let mut with_leftovers: Vec<TicketKey> = restore::list_all(&state)?
        .into_iter()
        .map(|r| r.ticket)
        .chain(overlays::list_all(&state)?.into_iter().map(|o| o.ticket))
        .collect();
    with_leftovers.sort();
    with_leftovers.dedup();
    let key = pick_ticket_to_deactivate(
        key,
        tickets::active(&state)?.map(|t| t.key),
        &with_leftovers,
    )?;

    let report = deactivate(
        &state,
        &ProcessRunner::new(),
        &key,
        manual_deactivation_target(status)?,
        now()?,
    )?;
    print_deactivation(&ui, &report)
}

// ------------------------------------------------------------ note, checks

pub fn note(key: TicketKey, text: String) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;
    require_tracked(&state, &key)?;
    if text.trim().is_empty() {
        bail!("A note cannot be empty");
    }
    notes::add_note(&state, &key, text.trim(), now()?)?;
    ui.success_item(&format!("Noted on {key}"), None)?;
    Ok(())
}

pub fn render_checklist(items: &[ChecklistItem]) -> Vec<String> {
    if items.is_empty() {
        return vec!["(empty)".into()];
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            format!(
                "{}. [{}] {}",
                i + 1,
                if item.done { "x" } else { " " },
                item.text
            )
        })
        .collect()
}

/// The item shown as number `number` (1-based).
pub fn item_at(items: &[ChecklistItem], number: usize) -> eyre::Result<&ChecklistItem> {
    number
        .checked_sub(1)
        .and_then(|i| items.get(i))
        .ok_or_else(|| {
            eyre!(
                "There is no checklist item {number} (there are {})",
                items.len()
            )
        })
}

pub fn check(command: CheckCommands) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let state = open_state()?;

    let key = match command {
        CheckCommands::Add { key, text } => {
            require_tracked(&state, &key)?;
            if text.trim().is_empty() {
                bail!("A checklist item cannot be empty");
            }
            notes::add_checklist_item(&state, &key, text.trim())?;
            key
        }
        CheckCommands::Toggle { key, number } => {
            require_tracked(&state, &key)?;
            let items = notes::list_checklist(&state, &key)?;
            let id = item_at(&items, number)?.id;
            notes::toggle_checklist_item(&state, id)?;
            key
        }
        CheckCommands::List { key } => {
            require_tracked(&state, &key)?;
            key
        }
    };

    for line in render_checklist(&notes::list_checklist(&state, &key)?) {
        ui.writeln(&line)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn deactivation_target_defaults_to_parked_and_refuses_integrated() {
        assert_eq!(
            manual_deactivation_target(None).unwrap(),
            LocalStatus::Parked
        );
        assert_eq!(
            manual_deactivation_target(Some(LocalStatus::Done)).unwrap(),
            LocalStatus::Done
        );
        assert_eq!(
            manual_deactivation_target(Some(LocalStatus::Reviewing)).unwrap(),
            LocalStatus::Reviewing
        );
        let err = manual_deactivation_target(Some(LocalStatus::Integrated)).unwrap_err();
        assert!(err.to_string().contains("only when it is merged and pushed"));
    }

    use super::*;
    use de_core::{activation::RepoRestore, git::StashRef, overlay::RevertOutcome};

    fn key(s: &str) -> TicketKey {
        s.parse().unwrap()
    }

    #[test]
    fn durations_are_compact() {
        assert_eq!(format_duration(0), "<1m");
        assert_eq!(format_duration(59), "<1m");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(45 * 60), "45m");
        assert_eq!(format_duration(3600), "1h 00m");
        assert_eq!(format_duration(3600 + 5 * 60 + 30), "1h 05m");
        assert_eq!(format_duration(26 * 3600), "26h 00m");
        assert_eq!(format_duration(-5), "<1m");
    }

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00");
        assert_eq!(format_timestamp(86_399), "1970-01-01 23:59");
        // Leap day, and a date after it.
        assert_eq!(format_timestamp(1_709_210_400), "2024-02-29 12:40");
        assert_eq!(format_timestamp(1_785_000_000), "2026-07-25 17:20");
        assert_eq!(format_timestamp(946_684_800), "2000-01-01 00:00");
    }

    #[test]
    fn list_rendering_aligns_and_handles_empty() {
        assert!(render_list(&[])[0].contains("de ticket claim"));

        let rows = [
            ListRow {
                key: "PROJ-1".into(),
                status: LocalStatus::Active,
                kind: TicketKind::Normal,
                seconds: 3900,
                title: Some("Fix the thing".into()),
            },
            ListRow {
                key: "LONGPROJ-123".into(),
                status: LocalStatus::Parked,
                kind: TicketKind::Hotfix,
                seconds: 0,
                title: None,
            },
        ];
        let lines = render_list(&rows);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("KEY"));
        assert!(lines[1].starts_with("PROJ-1        active"));
        assert!(lines[1].contains("1h 05m") && lines[1].ends_with("Fix the thing"));
        assert!(lines[2].contains("hotfix") && lines[2].ends_with("<1m"));
        // The status column starts at the same offset on every row.
        let offset = lines[0].find("STATUS").unwrap();
        assert!(lines[1][offset..].starts_with("active"));
        assert!(lines[2][offset..].starts_with("parked"));
        let kind = lines[0].find("KIND").unwrap();
        assert!(lines[1][kind..].starts_with("normal"), "{}", lines[1]);
        assert!(lines[2][kind..].starts_with("hotfix"), "{}", lines[2]);
        let time = lines[0].find("TIME").unwrap();
        assert!(lines[1][..time + 4].ends_with("1h 05m"), "{}", lines[1]);
        assert!(lines[2][..time + 4].ends_with("<1m"), "{}", lines[2]);
    }

    fn link_row(repo: &str, branch: Option<&str>, origin: RepoLinkOrigin) -> RepoLink {
        RepoLink {
            ticket: key("PROJ-1"),
            repo: repo.into(),
            branch: branch.map(String::from),
            origin,
        }
    }

    fn found(repo: &str, branches: &[&str]) -> RepoMatches {
        RepoMatches {
            repo: repo.into(),
            branches: branches.iter().map(|b| String::from(*b)).collect(),
        }
    }

    #[test]
    fn repo_lines_merge_links_with_live_branches() {
        let links = [
            link_row("api", Some("stale"), RepoLinkOrigin::Auto),
            link_row("web", Some("chosen"), RepoLinkOrigin::Manual),
            link_row("docs", None, RepoLinkOrigin::Excluded),
            link_row("gone", Some("old"), RepoLinkOrigin::Auto),
        ];
        let live = [
            found("api", &["feature/PROJ-1-x"]),
            found("web", &["a", "b"]),
            found("docs", &["feature/PROJ-1-docs"]),
            found("worker", &["PROJ-1"]),
        ];
        let lines = merge_repo_lines(&links, Some(&live));

        let by_repo: BTreeMap<&str, &RepoLine> =
            lines.iter().map(|l| (l.repo.as_str(), l)).collect();
        // Live branches replace a stale auto link; a manual choice wins; excluded stays hidden.
        assert_eq!(by_repo["api"].branches, ["feature/PROJ-1-x"]);
        assert_eq!(by_repo["web"].branches, ["chosen"]);
        assert_eq!(by_repo["web"].origin, Some(RepoLinkOrigin::Manual));
        assert!(by_repo["docs"].branches.is_empty());
        assert_eq!(by_repo["docs"].origin, Some(RepoLinkOrigin::Excluded));
        assert_eq!(by_repo["worker"].origin, None);
        assert_eq!(by_repo["worker"].branches, ["PROJ-1"]);
        // Without a workspace only the stored links are shown.
        let stored = merge_repo_lines(&links, None);
        assert_eq!(
            stored.iter().find(|l| l.repo == "api").unwrap().branches,
            ["stale"]
        );
        assert_eq!(stored.len(), 4);
        // A manual link without a branch takes the discovered ones.
        let unset = merge_repo_lines(
            &[link_row("web", None, RepoLinkOrigin::Manual)],
            Some(&[found("web", &["a"])]),
        );
        assert_eq!(unset[0].branches, ["a"]);
    }

    fn sample_show() -> ShowData {
        ShowData {
            key: key("PROJ-1"),
            title: Some("Fix login".into()),
            tracking: tickets::TicketTracking {
                key: key("PROJ-1"),
                status: LocalStatus::Active,
                manual_order: 0,
                kind_override: None,
                claimed_at: 0,
                updated_at: 0,
            },
            kind: TicketKind::Hotfix,
            repos: vec![
                RepoLine {
                    repo: "api".into(),
                    origin: Some(RepoLinkOrigin::Auto),
                    branches: vec!["feature/PROJ-1-x".into()],
                },
                RepoLine {
                    repo: "web".into(),
                    origin: None,
                    branches: vec!["a".into(), "b".into()],
                },
                RepoLine {
                    repo: "docs".into(),
                    origin: Some(RepoLinkOrigin::Excluded),
                    branches: vec![],
                },
            ],
            overlays: vec!["web".into()],
            checklist: vec![
                ChecklistItem {
                    id: 1,
                    ticket: key("PROJ-1"),
                    position: 0,
                    text: "Log in".into(),
                    done: true,
                },
                ChecklistItem {
                    id: 2,
                    ticket: key("PROJ-1"),
                    position: 1,
                    text: "Log out".into(),
                    done: false,
                },
            ],
            notes: vec![Note {
                id: 1,
                ticket: key("PROJ-1"),
                body: "flaky on Safari".into(),
                created_at: 1_709_210_400,
            }],
            seconds: 5400,
            timer_running: true,
            live: true,
        }
    }

    #[test]
    fn show_rendering_covers_every_section() {
        let text = render_show(&sample_show()).join("\n");
        assert!(
            text.starts_with("PROJ-1  [active]  hotfix  Fix login"),
            "{text}"
        );
        assert!(text.contains("time: 1h 30m (running)"));
        assert!(text.contains("api   feature/PROJ-1-x  (auto)"), "{text}");
        assert!(text.contains("web   a, b  <- several; choose with `de ticket link PROJ-1 web --branch <BRANCH>`  (found)"), "{text}");
        assert!(text.contains("docs  -  (excluded)"), "{text}");
        assert!(text.contains("test overlay applied in web"));
        assert!(text.contains("  1. [x] Log in\n  2. [ ] Log out"), "{text}");
        assert!(text.contains("2024-02-29 12:40  flaky on Safari"), "{text}");

        let mut bare = sample_show();
        bare.repos.clear();
        bare.notes.clear();
        bare.checklist.clear();
        bare.overlays.clear();
        bare.live = false;
        bare.timer_running = false;
        bare.kind = TicketKind::Normal;
        let text = render_show(&bare).join("\n");
        assert!(text.contains("no workspace to look at"));
        assert!(text.contains("(no repo has a branch"));
        assert!(text.contains("(empty)") && text.contains("(none)"));
        assert!(!text.contains("hotfix") && !text.contains("running"));
    }

    #[test]
    fn link_flags_are_checked() {
        assert_eq!(
            link_action(Some("b".into()), false).unwrap(),
            LinkAction::Manual(Some("b".into()))
        );
        assert_eq!(link_action(None, false).unwrap(), LinkAction::Manual(None));
        assert_eq!(link_action(None, true).unwrap(), LinkAction::Exclude);
        assert!(link_action(Some("b".into()), true).is_err());
    }

    #[test]
    fn baseline_is_only_asked_for_hotfixes() {
        let never = || -> eyre::Result<BaselineChoice> { panic!("must not ask") };

        assert_eq!(
            resolve_baseline(TicketKind::Normal, None, never).unwrap(),
            (BaselineChoice::Base, None)
        );
        assert_eq!(
            resolve_baseline(TicketKind::Normal, Some(BaselineChoice::Base), never).unwrap(),
            (BaselineChoice::Base, None)
        );
        let (choice, warning) =
            resolve_baseline(TicketKind::Normal, Some(BaselineChoice::Uat), never).unwrap();
        assert_eq!(choice, BaselineChoice::Base);
        assert!(warning.unwrap().contains("only applies to hotfixes"));

        // A hotfix takes the flag as is, and otherwise asks.
        assert_eq!(
            resolve_baseline(TicketKind::Hotfix, Some(BaselineChoice::Production), never).unwrap(),
            (BaselineChoice::Production, None)
        );
        assert_eq!(
            resolve_baseline(TicketKind::Hotfix, None, || Ok(BaselineChoice::Uat)).unwrap(),
            (BaselineChoice::Uat, None)
        );
        assert!(resolve_baseline(TicketKind::Hotfix, None, || Err(eyre!("no terminal"))).is_err());
    }

    #[test]
    fn the_ticket_to_deactivate_is_named_active_or_the_one_left_unfinished() {
        let (a, b) = (key("PROJ-1"), key("PROJ-2"));
        // Named wins, then the active one.
        assert_eq!(
            pick_ticket_to_deactivate(Some(b.clone()), Some(a.clone()), &[]).unwrap(),
            b
        );
        assert_eq!(
            pick_ticket_to_deactivate(None, Some(a.clone()), std::slice::from_ref(&b)).unwrap(),
            a
        );
        // Nothing active: the single ticket with leftovers (crash recovery).
        assert_eq!(
            pick_ticket_to_deactivate(None, None, std::slice::from_ref(&b)).unwrap(),
            b
        );
        let err = pick_ticket_to_deactivate(None, None, &[]).unwrap_err();
        assert!(err.to_string().contains("No ticket is active"));
        let err = pick_ticket_to_deactivate(None, None, &[a, b]).unwrap_err();
        assert!(err.to_string().contains("PROJ-1, PROJ-2"), "{err}");
    }

    #[test]
    fn enum_arguments_parse_from_text() {
        assert_eq!(
            "production".parse::<BaselineChoice>().unwrap(),
            BaselineChoice::Production
        );
        assert!("nope".parse::<BaselineChoice>().is_err());
        assert_eq!(
            "parked".parse::<LocalStatus>().unwrap(),
            LocalStatus::Parked
        );
        assert!("proj-1".parse::<TicketKey>().is_ok());
        assert!("not a key".parse::<TicketKey>().is_err());
    }

    #[test]
    fn activation_rendering_lists_each_repo() {
        use de_core::{activation::RepoActivation, git::FastForward};
        let report = ActivationReport {
            ticket: key("PROJ-1"),
            repos: vec![
                RepoActivation {
                    repo: "api".into(),
                    action: RepoAction::SwitchToTicketBranch("feature/PROJ-1-x".into()),
                    previous_branch: Some("develop".into()),
                    previous_commit: Some("0123456789".into()),
                    stash: Some(StashRef {
                        label: "de:PROJ-1:api".into(),
                        commit: "beef".into(),
                    }),
                    already_there: false,
                    fast_forward: None,
                    overlaid: vec![],
                    tasks_run: vec![],
                },
                RepoActivation {
                    repo: "web".into(),
                    action: RepoAction::FallBackToBaseline("develop".into()),
                    previous_branch: None,
                    previous_commit: Some("0123456789".into()),
                    stash: None,
                    already_there: false,
                    fast_forward: Some(FastForward::Diverged {
                        ahead: 1,
                        behind: 2,
                    }),
                    overlaid: vec!["acme/api-client".into()],
                    tasks_run: vec!["task build-ui".into()],
                },
            ],
            warnings: vec![],
        };
        let text = render_activation(&report).join("\n");
        assert!(
            text.contains("api  feature/PROJ-1-x  (ticket branch; was develop)"),
            "{text}"
        );
        assert!(text.contains("stashed local changes as 'de:PROJ-1:api'"));
        assert!(
            text.contains("web  develop  (baseline; was detached at 01234567)"),
            "{text}"
        );
        assert!(text.contains("diverged (ahead 1, behind 2); left as is"));
        assert!(text.contains("test overlay: acme/api-client"));
        assert!(text.contains("ran task build-ui"));
    }

    #[test]
    fn deactivation_rendering_never_hides_a_problem() {
        use de_core::activation::RestoreFailure;
        let report = DeactivationReport {
            ticket: key("PROJ-1"),
            overlays: vec![(
                "web".into(),
                RevertOutcome::Reverted {
                    lock_removed: false,
                    saved: Vec::new(),
                },
            )],
            repos: vec![
                RepoRestore {
                    repo: "api".into(),
                    restored_to: "develop".into(),
                    leftover_stash: Some(StashRef {
                        label: "de:PROJ-1:api:leftover".into(),
                        commit: "aa".into(),
                    }),
                    stash_popped: true,
                    stash_kept: None,
                },
                RepoRestore {
                    repo: "web".into(),
                    restored_to: "wip".into(),
                    leftover_stash: None,
                    stash_popped: false,
                    stash_kept: Some((
                        StashRef {
                            label: "de:PROJ-1:web".into(),
                            commit: "bb".into(),
                        },
                        "conflict in app.php\nsecond line".into(),
                    )),
                },
            ],
            failures: vec![RestoreFailure {
                repo: "docs".into(),
                error: "branch gone\nmore".into(),
            }],
            status: None,
        };
        let text = render_deactivation(&report).join("\n");
        assert!(text.contains("web: test overlay reverted"));
        assert!(text.contains("api: back on develop, your stashed changes are restored"));
        assert!(text.contains("stashed as 'de:PROJ-1:api:leftover'"));
        assert!(text.contains("stay in stash 'de:PROJ-1:web': conflict in app.php"));
        assert!(!text.contains("second line"));
        assert!(text.contains("docs: NOT restored: branch gone"));
    }

    #[test]
    fn checklist_rendering_and_numbering() {
        let items: Vec<ChecklistItem> = ["a", "b"]
            .iter()
            .enumerate()
            .map(|(i, t)| ChecklistItem {
                id: 10 + i as i64,
                ticket: key("PROJ-1"),
                position: i as i64,
                text: (*t).into(),
                done: i == 1,
            })
            .collect();
        assert_eq!(render_checklist(&items), ["1. [ ] a", "2. [x] b"]);
        assert_eq!(render_checklist(&[]), ["(empty)"]);

        assert_eq!(item_at(&items, 1).unwrap().id, 10);
        assert_eq!(item_at(&items, 2).unwrap().id, 11);
        assert!(item_at(&items, 0).is_err());
        assert!(item_at(&items, 3).is_err());
    }
}

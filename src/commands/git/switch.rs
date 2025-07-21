use std::{collections::HashSet, path::Path, process::Command};

use chrono::{DateTime, Utc};
use dialoguer::{Select, theme::ColorfulTheme};
use eyre::{Context, Result, eyre};
use itertools::Itertools;

use crate::{
    cli::OnDirtyAction,
    project::Project,
    types::Slug,
    utils::{
        git::{branch_exists, get_default_branch, run_git_command},
        ui::UserInterface,
    },
    workspace::{Workspace, WorkspaceProject},
};

pub fn switch(
    query: Option<String>,
    fallback: Option<String>,
    on_dirty: Option<OnDirtyAction>,
) -> Result<()> {
    let ui = UserInterface::new();

    ui.heading("Switch Branch")?;

    let workspace =
        Workspace::active()?.ok_or_else(|| eyre::eyre!("No active workspace found."))?;

    let target_branch = get_target_branch(&workspace, query)?;

    ui.info_item(&format!("Workspace: {}", workspace.config().name))?;
    ui.info_item(&format!("Target Branch: {target_branch}"))?;

    let dirty_projects = get_dirty_projects(&workspace)?;
    let action = on_dirty.unwrap_or(OnDirtyAction::Prompt);

    ui.new_line()?;
    ui.heading("Preflight")?;

    if !dirty_projects.is_empty() {
        handle_dirty_projects_preflight(&ui, &dirty_projects, &action)?;
    } else {
        ui.success_item("No dirty projects found. Proceeding...", None)?;
    }

    ui.new_line()?;
    ui.heading(&format!(
        "Synchronizing: {} (fallback: {})...",
        target_branch,
        fallback.as_deref().unwrap_or("default")
    ))?;

    let mut projects_with_issues = Vec::new();

    for (project_name, ws_project) in workspace.config().projects.iter() {
        let success = switch_project_branch(
            &ui,
            &workspace,
            ws_project,
            project_name,
            &target_branch,
            fallback.as_deref(),
            &action,
        )?;

        if !success {
            projects_with_issues.push(project_name.to_string());
        }
    }

    ui.new_line()?;
    ui.heading("Summary")?;

    if !projects_with_issues.is_empty() {
        ui.error_group(
            &format!("{} project(s) have issues:", projects_with_issues.len()),
            &projects_with_issues,
            None,
        )?;
    } else {
        ui.success_item("All projects synchronized successfully.", None)?;
    }

    Ok(())
}

fn switch_project_branch(
    ui: &UserInterface,
    workspace: &Workspace,
    ws_project: &WorkspaceProject,
    project_name: &Slug,
    target_branch: &str,
    fallback: Option<&str>,
    on_dirty: &OnDirtyAction,
) -> eyre::Result<bool> {
    ui.subheading(&format!(
        "{project_name} {}",
        ui.theme.dim(&format!("({})", ws_project.dir.display()))
    ))?;

    let project = Project::from_dir(&ws_project.dir)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to load project '{project_name}'"))?;

    ui.indented(|ui| {
        if !project.manifest().git.clone().unwrap_or_default().enabled {
            ui.info_item("Git is not enabled for this project. Skipping...")?;
            return Ok(true);
        }

        let dirty_result = handle_dirty_project(ui, &project, on_dirty)?;
        match dirty_result {
            DirtyResult::Proceed | DirtyResult::Stashed => {}
            DirtyResult::Skip | DirtyResult::StashFailed => {
                return Ok(true);
            }
        };

        let fallback_branch = if let Some(fallback) = fallback {
            fallback.to_string()
        } else if let Some(default_branch) = workspace.config().default_branch.as_deref() {
            default_branch.to_string()
        } else {
            get_default_branch(&ws_project.dir).unwrap_or_else(|_| "main".to_string())
        };

        let checkout_branch = if branch_exists(target_branch, &ws_project.dir)? {
            ui.info_item("Target branch found.")?;
            target_branch
        } else if branch_exists(&fallback_branch, &ws_project.dir)? {
            ui.warning_item(
                &format!(
                    "Target branch not found. Falling back to '{fallback_branch}'."
                ),
                None,
            )?;
            fallback_branch.as_str()
        } else {
            ui.warning_item(
                &format!(
                    "Neither target branch nor fallback branch '{fallback_branch}' found. Aborting."
                ),
                None,
            )?;
            return Ok(true);
        };

        if let Err(e) = run_git_command(&["checkout", checkout_branch], &ws_project.dir) {
            ui.error_item(&format!("Failed to switch branch: {e}"), None)?;
        } else {
            ui.success_item("Switched to target branch.", None)?;
        }

        // Restore stashed changes if it was stashed previously
        if let DirtyResult::Stashed = dirty_result {
            ui.info_item("Restoring stashed changes...")?;
            if let Err(e) = run_git_command(&["stash", "pop"], &ws_project.dir) {
                ui.error_item(&format!("Failed to restore stashed changes: {e}"), None)?;
                return Ok(false);
            } else {
                ui.success_item("Stashed changes restored successfully.", None)?;
            }
        }

        if is_project_dirty(&ws_project.dir)? {
            ui.error_item(
                &format!(
                    "{} detected. Please resolve manually.",
                    ui.theme.error("MERGE CONFLICT")
                ),
                None,
            )?;
            Ok(false)
        } else {
            Ok(true)
        }
    })
}

fn get_target_branch(workspace: &Workspace, query: Option<String>) -> Result<String> {
    if let Some(query) = query {
        get_target_branch_from_query(workspace, query)
    } else {
        unimplemented!()
    }
}

/// Fuzzy search through branches and return only match or use chosen branch
fn get_target_branch_from_query(workspace: &Workspace, query: String) -> Result<String> {
    use dialoguer::{Select, theme::ColorfulTheme};

    let branches = get_workspace_branches(workspace)?;

    // Fuzzy search: first, try for exact match (case-sensitive)
    if let Some(branch) = branches.iter().find(|b| b.name == query) {
        return Ok(branch.name.clone());
    }

    // Then, try for case-insensitive match
    if let Some(branch) = branches
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(&query))
    {
        return Ok(branch.name.clone());
    }

    // Then, try for substring match
    let matches: Vec<_> = branches
        .iter()
        .filter(|b| b.name.to_lowercase().contains(&query.to_lowercase()))
        .unique()
        .collect();

    if matches.len() == 1 {
        println!("Found one matching branch: {}", matches[0].name);
        Ok(matches[0].name.clone())
    } else if matches.is_empty() {
        return Err(eyre::eyre!("No branch found matching query '{}'", query));
    } else {
        let branch_names: Vec<_> = matches.iter().map(|b| b.name.clone()).collect();

        // Prompt user to select from matches
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Multiple branches match your query. Please select one:")
            .items(&branch_names)
            .default(0)
            .interact()?;

        return Ok(branch_names[selection].clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Branch {
    pub name: String,
    pub date: Option<DateTime<Utc>>,
}

fn get_workspace_branches(workspace: &Workspace) -> Result<Vec<Branch>> {
    let mut branches = HashSet::new();
    for project in workspace.config().projects.values() {
        let project_branches = get_project_branches(&project.dir)?;
        for branch in project_branches {
            branches.insert(branch);
        }
    }

    let mut branches: Vec<_> = branches.into_iter().collect();
    branches.dedup();
    branches.sort_by(|a, b| {
        // Sort by date if available, otherwise by name
        match (a.date, b.date) {
            (Some(date_a), Some(date_b)) => date_b.cmp(&date_a),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => a.name.cmp(&b.name),
        }
    });

    Ok(branches)
}

fn get_project_branches(dir: &Path) -> Result<Vec<Branch>, eyre::Error> {
    use chrono::{DateTime, Utc};

    let output = Command::new("git")
        .current_dir(dir)
        .arg("for-each-ref")
        .arg("--sort=-committerdate")
        .arg("refs/heads/")
        .arg("refs/remotes/")
        .arg("--format=%(committerdate:iso8601) %(refname:short)")
        .output()?;

    if !output.status.success() {
        return Err(eyre::eyre!(
            "Failed to list branches with commit dates: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mut seen = HashSet::new();
    let mut branches = Vec::new();

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Split into date and branch name
        if let Some((date_str, branch_name)) = line.split_at_checked(25) {
            let branch_name = branch_name.trim();

            // Remove "origin/HEAD" and similar symbolic refs
            if branch_name.ends_with("HEAD") {
                continue;
            }

            // Remove duplicate branches (local and remote with same name)
            let branch_name = if let Some(idx) = branch_name.find('/') {
                branch_name[idx + 1..].to_string()
            } else {
                branch_name.to_string()
            };

            if seen.contains(&branch_name) {
                continue;
            }

            seen.insert(branch_name.clone());

            // Parse date using carbon, convert to UTC chrono::DateTime
            let dt = DateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S %z")
                .map_err(|e| eyre::eyre!("Failed to parse date '{}': {}", date_str, e))?
                .with_timezone(&Utc);

            branches.push(Branch {
                name: branch_name,
                date: Some(dt),
            });
        }
    }

    Ok(branches)
}

fn get_dirty_projects(workspace: &Workspace) -> Result<Vec<String>> {
    let mut dirty_projects = Vec::new();
    for (project_name, ws_project) in workspace.config().projects.iter() {
        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load project '{project_name}'"))?;

        if !project.manifest().git.clone().unwrap_or_default().enabled {
            continue;
        }

        if is_project_dirty(&ws_project.dir)? {
            dirty_projects.push(project_name.to_string());
        }
    }
    Ok(dirty_projects)
}

fn is_project_dirty(dir: &std::path::Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("status")
        .arg("--porcelain")
        .output()?;
    Ok(!output.stdout.is_empty())
}

fn handle_dirty_projects_preflight(
    ui: &UserInterface,
    dirty_projects: &[String],
    on_dirty: &OnDirtyAction,
) -> Result<OnDirtyAction> {
    ui.warning_item("Uncommitted changes found in the following projects", None)?;

    ui.indented(|ui| {
        for project_name in dirty_projects {
            ui.info_item(&project_name.to_string())?;
        }

        match on_dirty {
            OnDirtyAction::Prompt => {
                let selections = &[
                    "Prompt individually",
                    "Stash changes and proceed",
                    "Force checkout (discard all changes)",
                    "Abort operation",
                ];

                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("What would you like to do?")
                    .default(0)
                    .items(&selections[..])
                    .interact()?;

                match selection {
                    0 => Ok(OnDirtyAction::Prompt),
                    1 => Ok(OnDirtyAction::Stash),
                    2 => Ok(OnDirtyAction::Force),
                    _ => Err(eyre::eyre!("Operation aborted.")),
                }
            }
            n @ OnDirtyAction::Stash => {
                ui.warning_item("Stash changes and proceed", None)?;
                Ok(*n)
            }
            n @ OnDirtyAction::Force => {
                ui.warning_item("Force checkout (discard all changes)", None)?;
                Ok(*n)
            }
            OnDirtyAction::Abort => Err(eyre::eyre!("Operation aborted.")),
        }
    })
}

enum DirtyResult {
    Proceed,
    Skip,
    Stashed,
    StashFailed,
}

fn handle_dirty_project(
    ui: &UserInterface,
    project: &Project,
    on_dirty: &OnDirtyAction,
) -> eyre::Result<DirtyResult> {
    if !is_project_dirty(project.dir())? {
        return Ok(DirtyResult::Proceed);
    };

    fn stash_changes(ui: &UserInterface, project: &Project) -> eyre::Result<DirtyResult> {
        ui.info_item("Stashing changes...")?;
        if let Err(e) = run_git_command(&["stash", "push", "-u"], project.dir()) {
            ui.error_item(&format!("Failed to stash changes: {e}"), None)?;
            return Ok(DirtyResult::StashFailed);
        }
        ui.success_item("Changes stashed successfully.", None)?;
        Ok(DirtyResult::Stashed)
    }

    fn force_checkout(ui: &UserInterface, project: &Project) -> eyre::Result<DirtyResult> {
        ui.warning_item("Forcing checkout, discarding all changes...", None)?;
        run_git_command(&["checkout", "--force"], project.dir())?;
        ui.success_item("Checkout forced successfully.", None)?;
        Ok(DirtyResult::Proceed)
    }

    match on_dirty {
        OnDirtyAction::Prompt => {
            let selections = &[
                "Stash changes and proceed",
                "Force checkout (discard all changes)",
                "Skip this project and continue with others",
                "Abort operation",
            ];

            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Uncommitted changes found in project '{}'",
                    project.manifest().project().name
                ))
                .default(0)
                .items(&selections[..])
                .interact()?;

            match selection {
                0 => {
                    stash_changes(ui, project)?;
                    Ok(DirtyResult::Proceed)
                }
                1 => {
                    force_checkout(ui, project)?;
                    Ok(DirtyResult::Proceed)
                }
                2 => Ok(DirtyResult::Skip),
                _ => Err(eyre::eyre!("Operation aborted by user.")),
            }
        }
        OnDirtyAction::Stash => {
            stash_changes(ui, project)?;
            Ok(DirtyResult::Proceed)
        }
        OnDirtyAction::Force => {
            force_checkout(ui, project)?;
            Ok(DirtyResult::Proceed)
        }
        OnDirtyAction::Abort => Err(eyre::eyre!("Operation aborted by user.")),
    }
}

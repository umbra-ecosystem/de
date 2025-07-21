use std::{collections::HashSet, path::Path, process::Command};

use chrono::{DateTime, Utc};
use dialoguer::{Select, theme::ColorfulTheme};
use eyre::{Context, Result, eyre};
use itertools::Itertools;

use crate::{
    cli::OnDirtyAction,
    project::Project,
    utils::{
        git::{branch_exists, get_default_branch, run_git_command},
        ui::UserInterface,
    },
    workspace::Workspace,
};

pub fn switch(
    query: Option<String>,
    fallback: Option<String>,
    on_dirty: Option<OnDirtyAction>,
) -> Result<()> {
    let ui = UserInterface::new();
    let theme = &ui.theme;

    ui.heading("Switching Workspace Branch")?;

    let workspace =
        Workspace::active()?.ok_or_else(|| eyre::eyre!("No active workspace found."))?;

    let target_branch = get_target_branch(&workspace, query)?;

    ui.info_item(&format!("Workspace: {}", workspace.config().name))?;
    ui.info_item(&format!("Target Branch: {}", target_branch))?;

    let dirty_projects = get_dirty_projects(&workspace)?;
    let action = on_dirty.unwrap_or(OnDirtyAction::Prompt);

    ui.heading("Preflight checks:")?;

    if !dirty_projects.is_empty() {
        handle_dirty_projects(&ui, &dirty_projects, &action)?;
    } else {
        ui.success_item("No dirty projects found. Proceeding...", None)?;
    }

    println!(
        "{}",
        theme.highlight(&format!(
            "Synchronizing workspace to branch \'{}\' (fallback: \'{}\')...",
            target_branch,
            fallback.as_deref().unwrap_or("default")
        ))
    );

    let mut projects_with_issues = Vec::new();

    for (project_name, ws_project) in workspace.config().projects.iter() {
        let mut messages = Vec::new();
        let mut has_issue = false;
        let stashed =
            dirty_projects.contains(&project_name.to_string()) && action == OnDirtyAction::Stash;

        messages.push(theme.highlight(&format!("  - Project: {project_name}")));

        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load project '{project_name}'"))?;

        if !project.manifest().git.clone().unwrap_or_default().enabled {
            messages.push(theme.warn("  Git is not enabled for this project. Skipping..."));
            continue;
        }

        if stashed {
            messages.push(theme.highlight("  Stashing changes..."));
            if let Err(e) = run_git_command(&["stash", "push", "-u"], &ws_project.dir) {
                messages.push(theme.error(&format!("  STASH FAILED: {e}")));
                has_issue = true;
            }
        }

        let fallback_branch = if let Some(fallback) = fallback.as_deref() {
            fallback.to_string()
        } else if let Some(default_branch) = workspace.config().default_branch.as_deref() {
            default_branch.to_string()
        } else {
            get_default_branch(&ws_project.dir).unwrap_or_else(|_| "main".to_string())
        };

        let checkout_branch = if branch_exists(&target_branch, &ws_project.dir)? {
            messages.push(theme.highlight(&format!("  Target branch \'{target_branch}\' found.")));
            &target_branch
        } else {
            messages.push(theme.warn(&format!(
                "  Target branch \'{target_branch}\' not found. Falling back to \'{fallback_branch}\'."
            )));
            &fallback_branch
        };

        let mut args = vec!["checkout"];
        if action == OnDirtyAction::Force {
            args.push("--force");
            messages.push(theme.warn("  Forcing checkout..."));
        }
        args.push(checkout_branch);

        if let Err(e) = run_git_command(&args, &ws_project.dir) {
            messages.push(theme.error(&format!("  CHECKOUT FAILED: {e}")));
            has_issue = true;
        } else {
            messages.push(theme.success(&format!("  Switched to \'{checkout_branch}\'.")));
        }

        if stashed {
            messages.push(theme.highlight("  Restoring stashed changes..."));
            if let Err(e) = run_git_command(&["stash", "pop"], &ws_project.dir) {
                messages.push(theme.error(&format!("  STASH POP FAILED: {e}")));
                has_issue = true;
            }
        }

        if is_project_dirty(&ws_project.dir)? {
            messages.push(theme.warn("  MERGE CONFLICT detected. Please resolve manually."));
            has_issue = true;
        }

        for message in messages {
            println!("{message}");
        }

        if has_issue {
            projects_with_issues.push(project_name.to_string());
        }
    }

    println!();

    if !projects_with_issues.is_empty() {
        println!(
            "{}",
            theme.error(&format!(
                "{} project(s) have issues:",
                projects_with_issues.len()
            ))
        );
        for project_name in projects_with_issues {
            println!("  - {}", theme.error(&project_name));
        }
    } else {
        println!(
            "{}",
            theme.success("All projects synchronized successfully.")
        );
    }

    Ok(())
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

fn handle_dirty_projects(
    ui: &UserInterface,
    dirty_projects: &[String],
    on_dirty: &OnDirtyAction,
) -> Result<()> {
    ui.warning_item("Uncommitted changes found in the following projects", None)?;

    ui.indented(|ui| {
        for project_name in dirty_projects {
            ui.info_item(&format!("{project_name}"))?;
        }

        match on_dirty {
            OnDirtyAction::Prompt => {
                let selections = &[
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
                    0 => {}
                    1 => {}
                    _ => return Err(eyre::eyre!("Operation aborted.")),
                }
            }
            OnDirtyAction::Stash => ui.warning_item("Stashing changes...", None)?,
            OnDirtyAction::Force => {
                ui.warning_item("Forcing checkout, discarding all changes...", None)?
            }
            OnDirtyAction::Abort => return Err(eyre::eyre!("Operation aborted.")),
        }

        Ok(())
    })?;

    Ok(())
}

use std::{collections::HashSet, path::Path, process::Command};

use chrono::{DateTime, Utc};
use dialoguer::{Select, theme::ColorfulTheme};
use eyre::Result;

use crate::{cli::OnDirtyAction, utils::theme::Theme, workspace::Workspace};

pub fn switch(
    query: Option<String>,
    fallback: Option<String>,
    on_dirty: Option<OnDirtyAction>,
) -> Result<()> {
    let theme = Theme::new();
    let workspace =
        Workspace::active()?.ok_or_else(|| eyre::eyre!("No active workspace found."))?;

    let target_branch = get_target_branch(&workspace, query)?;

    let dirty_projects = get_dirty_projects(&workspace)?;
    let action = on_dirty.unwrap_or(OnDirtyAction::Prompt);

    if !dirty_projects.is_empty() {
        handle_dirty_projects(&dirty_projects, &action, &theme)?;
    }

    println!(
        "{}",
        theme.info(&format!(
            "Synchronizing workspace to branch \'{}\' (fallback: \'{}\')...",
            target_branch,
            fallback.as_deref().unwrap_or("default")
        ))
    );

    let mut projects_with_issues = Vec::new();

    for (project_name, project) in workspace.config().projects.iter() {
        let mut messages = Vec::new();
        let mut has_issue = false;
        let stashed =
            dirty_projects.contains(&project_name.to_string()) && action == OnDirtyAction::Stash;

        messages.push(theme.info(&format!("  - Project: {}", project_name)));

        if stashed {
            messages.push(theme.info("    Stashing changes..."));
            if let Err(e) = run_git_command(&["stash", "push", "-u"], &project.dir) {
                messages.push(theme.error(&format!("    STASH FAILED: {}", e)));
                has_issue = true;
            }
        }

        let fallback_branch = fallback.clone().unwrap_or_else(|| {
            get_default_branch(&project.dir).unwrap_or_else(|_| "main".to_string())
        });

        let checkout_branch = if branch_exists(&target_branch, &project.dir)? {
            messages.push(theme.info(&format!("    Target branch \'{}\' found.", target_branch)));
            &target_branch
        } else {
            messages.push(theme.warn(&format!(
                "    Target branch \'{}\' not found. Falling back to \'{}\'.",
                target_branch, fallback_branch
            )));
            &fallback_branch
        };

        let mut args = vec!["checkout"];
        if action == OnDirtyAction::Force {
            args.push("--force");
            messages.push(theme.warn("    Forcing checkout..."));
        }
        args.push(checkout_branch);

        if let Err(e) = run_git_command(&args, &project.dir) {
            messages.push(theme.error(&format!("    CHECKOUT FAILED: {}", e)));
            has_issue = true;
        } else {
            messages.push(theme.success(&format!("    Switched to \'{}\'.", checkout_branch)));
        }

        if stashed {
            messages.push(theme.info("    Restoring stashed changes..."));
            if let Err(e) = run_git_command(&["stash", "pop"], &project.dir) {
                messages.push(theme.error(&format!("    STASH POP FAILED: {}", e)));
                has_issue = true;
            }
        }

        if is_project_dirty(&project.dir)? {
            messages.push(theme.warn("    MERGE CONFLICT detected. Please resolve manually."));
            has_issue = true;
        }

        for message in messages {
            println!("{}", message);
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
        .collect();

    if matches.len() == 1 {
        println!("Found one matching branch: {}", matches[0].name);
        return Ok(matches[0].name.clone());
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
    for (project_name, project) in workspace.config().projects.iter() {
        if is_project_dirty(&project.dir)? {
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
    dirty_projects: &[String],
    on_dirty: &OnDirtyAction,
    theme: &Theme,
) -> Result<()> {
    println!(
        "{}",
        theme.warn("Uncommitted changes found in the following projects:")
    );

    for project_name in dirty_projects {
        println!("  - {}", project_name);
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
                0 => println!("Stashing changes..."),
                1 => println!("Forcing checkout..."),
                _ => return Err(eyre::eyre!("Operation aborted.")),
            }
        }
        OnDirtyAction::Stash => println!("Stashing changes..."),
        OnDirtyAction::Force => println!("Forcing checkout..."),
        OnDirtyAction::Abort => return Err(eyre::eyre!("Operation aborted.")),
    }

    Ok(())
}

fn run_git_command(args: &[&str], dir: &std::path::Path) -> Result<()> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    for arg in args {
        command.arg(arg);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(eyre::eyre!(
            "Git command failed: {}\n{}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn branch_exists(branch: &str, dir: &std::path::Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("branch")
        .arg("--list")
        .arg(branch)
        .output()?;
    let remote_output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("branch")
        .arg("-r")
        .arg("--list")
        .arg(format!("origin/{}", branch))
        .output()?;
    Ok(!output.stdout.is_empty() || !remote_output.stdout.is_empty())
}

fn get_default_branch(dir: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("origin/HEAD")
        .output()?;
    if !output.status.success() {
        return Err(eyre::eyre!("Failed to get default branch"));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string()
        .replace("origin/", ""))
}

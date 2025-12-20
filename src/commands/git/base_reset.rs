use crate::{
    cli::OnDirtyAction,
    project::Project,
    utils::{
        formatter::Formatter,
        git::{
            branch_exists, get_current_branch, get_default_branch, has_unpushed_commits,
            is_project_dirty, run_git_command,
        },
        theme::Theme,
    },
    workspace::Workspace,
};
use dialoguer::{Select, theme::ColorfulTheme};
use eyre::{Context, Result, eyre};

pub fn base_reset(base_branch: Option<String>, on_dirty: OnDirtyAction) -> Result<()> {
    // Check if we have a workspace or should work on inferred project
    let workspace_opt = Workspace::active()?;

    if let Some(workspace) = workspace_opt {
        // Workspace mode - reset all projects
        reset_workspace(workspace, base_branch, on_dirty)
    } else {
        // Inferred mode - reset current project only
        let project = Project::current_or_inferred()?
            .ok_or_else(|| eyre::eyre!("No project found in current directory"))?;

        if project.is_inferred() {
            let theme = Theme::new();
            println!(
                "{}",
                theme.dim("💡 No de.toml found - working in inferred mode")
            );
        }

        // Determine the branch to use
        let branch = if let Some(branch) = base_branch.as_deref() {
            branch.to_string()
        } else {
            // Auto-detect default branch from git
            get_default_branch(project.dir()).unwrap_or_else(|_| "main".to_string())
        };

        let theme = Theme::new();
        println!(
            "{}",
            theme.highlight(&format!("Resetting project to base branch '{branch}'..."))
        );

        // Use the extracted function
        let has_issue = reset_single_project(&project, &branch, on_dirty, false)?;

        // Summary
        println!();
        let formatter = Formatter::new();
        formatter.heading("Summary:")?;

        if !has_issue {
            println!(
                "{}",
                theme.success(&format!(
                    "✓ Project is ready for new feature branch on '{branch}'."
                ))
            );
        } else {
            println!(
                "{}",
                theme.error(
                    "⚠ Project could not be fully prepared. Please review the errors above."
                )
            );
        }

        Ok(())
    }
}

/// Reset all projects in a workspace to base branch
fn reset_workspace(
    workspace: Workspace,
    base_branch: Option<String>,
    on_dirty: OnDirtyAction,
) -> Result<()> {
    let theme = Theme::new();
    let formatter = Formatter::new();

    // Determine the branch to use
    let branch = if let Some(branch) = base_branch.as_deref() {
        branch
    } else {
        workspace
            .config()
            .default_branch
            .as_deref()
            .ok_or_else(|| eyre!("No default branch set in workspace config. Set default branch in workspace config or provide a base branch to command"))?
    };

    println!(
        "{}",
        theme.highlight(&format!("Resetting workspace to base branch '{branch}'..."))
    );

    let mut projects_with_issues = Vec::new();
    let mut projects_ready = Vec::new();

    let mut aborted = false;
    for (project_name, ws_project) in workspace.config().projects.iter() {
        if aborted {
            break;
        }

        // Print project header with name, path, and branch (colorized)
        println!();
        println!(
            "Project: {} {}{}{}",
            theme.accent(project_name.as_str()),
            theme.dim("("),
            theme.dim(&ws_project.dir.display().to_string()),
            theme.dim(")")
        );

        let project = Project::from_dir(&ws_project.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to load project '{project_name}'"))?;

        if !project.manifest().git.clone().unwrap_or_default().enabled {
            println!(" Git is not enabled for this project. Skipping...");
            continue;
        }

        // Use the extracted function for resetting single project
        match reset_single_project(&project, branch, on_dirty, true) {
            Ok(has_issue) => {
                if !has_issue {
                    projects_ready.push(project_name.to_string());
                } else {
                    projects_with_issues.push(project_name.to_string());
                }
            }
            Err(e) => {
                // Check if user aborted
                if e.to_string().contains("aborted") {
                    aborted = true;
                    break;
                }
                projects_with_issues.push(project_name.to_string());
            }
        }
    }

    println!();
    formatter.heading("Summary:")?;

    if aborted {
        println!(
            "{}",
            theme.error("Command aborted by user. Some projects may not have been processed.")
        );
    }

    if !projects_with_issues.is_empty() {
        println!(
            "{}",
            theme.error(&format!(
                "{} project(s) could not be prepared:",
                projects_with_issues.len()
            ))
        );
        for project_name in projects_with_issues.clone() {
            println!("  - {}", theme.error(&project_name));
        }
    }

    if !aborted && projects_ready.is_empty() && projects_with_issues.is_empty() {
        println!("{}", theme.warn("No projects were prepared."));
    }

    if !aborted && !projects_ready.is_empty() && projects_with_issues.is_empty() {
        println!(
            "{}",
            theme.success("All projects are ready for new feature branch.")
        );
    }

    Ok(())
}

/// Reset a single project to base branch
/// Returns Ok(has_issue) where has_issue is true if there were problems
/// in_workspace indicates if we're processing multiple projects (affects prompt choices)
fn reset_single_project(
    project: &Project,
    branch: &str,
    on_dirty: OnDirtyAction,
    in_workspace: bool,
) -> Result<bool> {
    let theme = Theme::new();

    if let Ok(current_branch) = get_current_branch(project.dir()) {
        println!(
            "  Current branch: {}",
            theme.accent(current_branch.as_str())
        );
    }

    let mut has_issue = false;

    // 1. Fetch all remotes
    println!("  Fetching remotes...");
    if let Err(e) = run_git_command(&["fetch", "--all", "--prune"], project.dir()) {
        println!(
            "  {} {}",
            theme.error("FETCH FAILED:"),
            theme.highlight(&e.to_string())
        );
        has_issue = true;
    }

    // 1b. Check for unpushed commits
    if let Ok(current_branch) = get_current_branch(project.dir())
        && let Ok(true) = has_unpushed_commits(&current_branch, project.dir())
    {
        println!("  {}", theme.warn("You have unpushed commits!"));
        let choices = if in_workspace {
            vec![
                "Push commits now",
                "Skip this project",
                "Abort all (stop processing)",
                "Proceed anyway (dangerous!)",
            ]
        } else {
            vec![
                "Push commits now",
                "Abort operation",
                "Proceed anyway (dangerous!)",
            ]
        };
        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What do you want to do?")
            .default(0)
            .items(&choices)
            .interact()?;
        match selection {
            0 => {
                // Try to push
                if let Err(e) = run_git_command(&["push"], project.dir()) {
                    println!(
                        "  {} {}",
                        theme.error("PUSH FAILED:"),
                        theme.highlight(&e.to_string())
                    );
                    has_issue = true;
                }
            }
            1 => {
                if in_workspace {
                    return Ok(true); // Skip this project
                } else {
                    return Err(eyre!("Operation aborted by user."));
                }
            }
            2 => {
                if in_workspace {
                    return Err(eyre!("Operation aborted by user."));
                } else {
                    // Proceed anyway
                }
            }
            3 => {} // Proceed anyway (workspace mode only)
            _ => unreachable!(),
        }
    }

    // 2. Check for uncommitted changes
    let dirty = is_project_dirty(project.dir()).unwrap_or(false);
    let mut action = on_dirty;

    if dirty {
        match action {
            OnDirtyAction::Prompt => {
                println!("  {}", theme.warn("Uncommitted changes detected!"));

                let choices = if in_workspace {
                    vec![
                        "Stash changes and proceed",
                        "Force reset (discard all changes)",
                        "Skip this project",
                        "Abort all (stop processing)",
                    ]
                } else {
                    vec![
                        "Stash changes and proceed",
                        "Force reset (discard all changes)",
                        "Abort operation",
                    ]
                };

                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("What do you want to do?")
                    .default(0)
                    .items(&choices)
                    .interact()?;

                match selection {
                    0 => action = OnDirtyAction::Stash,
                    1 => action = OnDirtyAction::Force,
                    2 => {
                        if in_workspace {
                            return Ok(true); // Skip this project
                        } else {
                            return Err(eyre!("Operation aborted by user."));
                        }
                    }
                    3 => return Err(eyre!("Operation aborted by user.")),
                    _ => unreachable!(),
                }
            }
            OnDirtyAction::Stash => {
                println!(
                    "  {} {}",
                    theme.warn("Uncommitted changes detected —"),
                    theme.highlight("stashing changes")
                );
            }
            OnDirtyAction::Force => {
                println!(
                    "  {} {}",
                    theme.warn("Uncommitted changes detected —"),
                    theme.highlight("discarding all local changes")
                );
            }
            OnDirtyAction::Abort => {
                return Err(eyre!("Operation aborted by user."));
            }
        }
    }

    // Handle dirty actions
    if dirty {
        match action {
            OnDirtyAction::Stash => {
                println!("  Stashing changes...");
                if let Err(e) = run_git_command(&["stash", "push", "-u"], project.dir()) {
                    println!(
                        "  {} {}",
                        theme.error("STASH FAILED:"),
                        theme.highlight(&e.to_string())
                    );
                    has_issue = true;
                }
            }
            OnDirtyAction::Force => {
                println!("  Discarding all local changes...");
                if let Err(e) = run_git_command(&["reset", "--hard"], project.dir()) {
                    println!(
                        "  {} {}",
                        theme.error("RESET FAILED:"),
                        theme.highlight(&e.to_string())
                    );
                    has_issue = true;
                }
            }
            OnDirtyAction::Abort | OnDirtyAction::Prompt => {}
        }
    } else {
        println!("  {}", theme.success("Working directory clean."));
    }

    // 3. Checkout the base branch
    println!("  Checking out branch {}...", theme.highlight(branch));
    if !branch_exists(branch, project.dir())? {
        // Try to check out from remote if not present locally
        let remote_branch = format!("origin/{branch}");
        if branch_exists(&remote_branch, project.dir())? {
            if let Err(e) =
                run_git_command(&["checkout", "-B", branch, &remote_branch], project.dir())
            {
                println!(
                    "  {} {}",
                    theme.error("CHECKOUT FAILED:"),
                    theme.highlight(&e.to_string())
                );
                has_issue = true;
            } else {
                println!(
                    "  {} {} {}",
                    theme.success("Checked out"),
                    theme.highlight(branch),
                    theme.success("from remote.")
                );
            }
        } else {
            println!("  {} {}", theme.error("Branch"), theme.highlight(branch),);
            println!("    {}", theme.error("not found locally or on remote."));
            has_issue = true;
        }
    } else if let Err(e) = run_git_command(&["checkout", branch], project.dir()) {
        println!(
            "  {} {}",
            theme.error("CHECKOUT FAILED:"),
            theme.highlight(&e.to_string())
        );
        has_issue = true;
    } else {
        println!(
            "  {} {}",
            theme.success("Checked out"),
            theme.highlight(branch)
        );
    }

    // 4. Reset hard to remote branch
    println!(
        "  Resetting to {}...",
        theme.highlight(&format!("origin/{branch}"))
    );
    if let Err(e) = run_git_command(
        &["reset", "--hard", &format!("origin/{branch}")],
        project.dir(),
    ) {
        println!(
            "  {} {}",
            theme.error("RESET FAILED:"),
            theme.highlight(&e.to_string())
        );
        has_issue = true;
    } else {
        println!("  {}", theme.success("Reset complete."));
    }

    // 5. Clean untracked files
    println!("  Cleaning untracked files...");
    if let Err(e) = run_git_command(&["clean", "-fd"], project.dir()) {
        println!(
            "  {} {}",
            theme.error("CLEAN FAILED:"),
            theme.highlight(&e.to_string())
        );
        has_issue = true;
    } else {
        println!("  {}", theme.success("Clean complete."));
    }

    // 6. Final status (only in workspace mode)
    if in_workspace {
        if !has_issue {
            println!(
                "  {} {} ",
                theme.success("Ready for"),
                theme.highlight("new feature branch.")
            );
        }
    }

    Ok(has_issue)
}

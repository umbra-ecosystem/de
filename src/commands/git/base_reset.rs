use crate::{
    cli::OnDirtyAction,
    project::Project,
    utils::{
        formatter::Formatter,
        git::{
            branch_exists, get_current_branch, has_unpushed_commits, is_project_dirty,
            run_git_command,
        },
        theme::Theme,
    },
    workspace::Workspace,
};
use dialoguer::{Select, theme::ColorfulTheme};
use eyre::{Context, Result, eyre};

pub fn base_reset(base_branch: Option<String>, on_dirty: OnDirtyAction) -> Result<()> {
    let theme = Theme::new();
    let formatter = Formatter::new();
    let workspace =
        Workspace::active()?.ok_or_else(|| eyre::eyre!("No active workspace found."))?;

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

        if let Ok(current_branch) = get_current_branch(&ws_project.dir) {
            println!(
                "  Current branch: {}",
                theme.accent(current_branch.as_str())
            );
        }

        // 1. Fetch all remotes
        println!("  Fetching remotes...");
        let mut has_issue = false;
        if let Err(e) = run_git_command(&["fetch", "--all", "--prune"], &ws_project.dir) {
            println!(
                "  {} {}",
                theme.error("FETCH FAILED:"),
                theme.highlight(&e.to_string())
            );
            has_issue = true;
        }

        // 1b. Check for unpushed commits
        if let Ok(current_branch) = get_current_branch(&ws_project.dir)
            && let Ok(true) = has_unpushed_commits(&current_branch, &ws_project.dir)
        {
            println!("  {}", theme.warn("You have unpushed commits!"));
            let choices = &[
                "Push commits now",
                "Skip this project",
                "Abort all (stop processing)",
                "Proceed anyway (dangerous!)",
            ];
            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("What do you want to do?")
                .default(0)
                .items(choices)
                .interact()?;
            match selection {
                0 => {
                    // Try to push
                    if let Err(e) = run_git_command(&["push"], &ws_project.dir) {
                        println!(
                            "  {} {}",
                            theme.error("PUSH FAILED:"),
                            theme.highlight(&e.to_string())
                        );
                        has_issue = true;
                    }
                }
                1 => continue,
                2 => {
                    aborted = true;
                    break;
                }
                3 => {} // Proceed anyway
                _ => unreachable!(),
            }
        }

        // 2. Check for uncommitted changes
        let dirty = is_project_dirty(&ws_project.dir).unwrap_or(false);
        let mut action = on_dirty;
        let mut skip_project = false;
        let mut abort_all = false;

        if dirty {
            match action {
                OnDirtyAction::Prompt => {
                    println!("  {}", theme.warn("Uncommitted changes detected!"));

                    let choices = &[
                        "Stash changes and proceed",
                        "Force reset (discard all changes)",
                        "Skip this project",
                        "Abort all (stop processing)",
                    ];

                    let selection = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("What do you want to do?")
                        .default(0)
                        .items(choices)
                        .interact()?;

                    match selection {
                        0 => action = OnDirtyAction::Stash,
                        1 => action = OnDirtyAction::Force,
                        2 => skip_project = true,
                        3 => abort_all = true,
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
                    println!(
                        "  {} {}",
                        theme.error("Uncommitted changes detected —"),
                        theme.highlight("aborted by user")
                    );
                    abort_all = true;
                }
            }
        }

        if skip_project {
            continue;
        }
        if abort_all {
            aborted = true;
            break;
        }

        // Handle dirty actions
        if dirty {
            match action {
                OnDirtyAction::Stash => {
                    println!("  Stashing changes...");
                    if let Err(e) = run_git_command(&["stash", "push", "-u"], &ws_project.dir) {
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
                    if let Err(e) = run_git_command(&["reset", "--hard"], &ws_project.dir) {
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
        if !branch_exists(branch, &ws_project.dir)? {
            // Try to check out from remote if not present locally
            let remote_branch = format!("origin/{branch}");
            if branch_exists(&remote_branch, &ws_project.dir)? {
                if let Err(e) =
                    run_git_command(&["checkout", "-B", branch, &remote_branch], &ws_project.dir)
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
        } else if let Err(e) = run_git_command(&["checkout", branch], &ws_project.dir) {
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
            &ws_project.dir,
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
        if let Err(e) = run_git_command(&["clean", "-fd"], &ws_project.dir) {
            println!(
                "  {} {}",
                theme.error("CLEAN FAILED:"),
                theme.highlight(&e.to_string())
            );
            has_issue = true;
        } else {
            println!("  {}", theme.success("Clean complete."));
        }

        // 6. Final status
        if !has_issue {
            println!(
                "  {} {} ",
                theme.success("Ready for"),
                theme.highlight("new feature branch.")
            );
            projects_ready.push(project_name.to_string());
        } else {
            projects_with_issues.push(project_name.to_string());
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

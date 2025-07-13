
use crate::{
    cli::OnDirtyAction, utils::formatter::Formatter, utils::theme::Theme, workspace::Workspace,
};
use dialoguer::{Select, theme::ColorfulTheme};
use eyre::Result;
use std::process::Command;

pub fn base_reset(base_branch: Option<String>, on_dirty: OnDirtyAction) -> Result<()> {
    let theme = Theme::new();
    let formatter = Formatter::new();
    let workspace =
        Workspace::active()?.ok_or_else(|| eyre::eyre!("No active workspace found."))?;

    // Determine the branch to use
    let branch = if let Some(branch) = base_branch {
        branch
    } else {
        // Use workspace default branch or fallback to "dev"
        let first_project = workspace.config().projects.values().next();
        if let Some(project) = first_project {
            get_default_branch(&project.dir).unwrap_or_else(|_| "dev".to_string())
        } else {
            "dev".to_string()
        }
    };

    println!(
        "{}",
        theme.highlight(&format!(
            "Resetting workspace to base branch '{}'...",
            branch
        ))
    );

    let mut projects_with_issues = Vec::new();
    let mut projects_ready = Vec::new();

    let mut aborted = false;
    for (project_name, project) in workspace.config().projects.iter() {
        if aborted {
            break;
        }

        // Print project header with name, path, and branch (colorized)
        println!();
        println!(
            "Project: {} {}{}{}",
            theme.accent(project_name.as_str()),
            theme.dim("("),
            theme.dim(&project.dir.display().to_string()),
            theme.dim(")")
        );
        if let Ok(current_branch) = get_current_branch(&project.dir) {
            println!(
                "  Current branch: {}",
                theme.accent(current_branch.as_str())
            );
        }

        // 1. Fetch all remotes
        println!("  Fetching remotes...");
        let mut has_issue = false;
        if let Err(e) = run_git_command(&["fetch", "--all", "--prune"], &project.dir) {
            println!(
                "  {} {}",
                theme.error("FETCH FAILED:"),
                theme.highlight(&e.to_string())
            );
            has_issue = true;
        }

        // 2. Check for uncommitted changes
        let dirty = is_project_dirty(&project.dir).unwrap_or(false);
        let mut action = on_dirty;
        let mut skip_project = false;
        let mut abort_all = false;

        if dirty {
            // Not prompting, just print the action
            match action {
                OnDirtyAction::Prompt => {
                    println!("  {}", theme.warn("Uncommitted changes detected — "));

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
                    if let Err(e) = run_git_command(&["stash", "push", "-u"], &project.dir) {
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
                    if let Err(e) = run_git_command(&["reset", "--hard"], &project.dir) {
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
        println!(
            "  Checking out branch {}...",
            theme.highlight(branch.as_str())
        );
        if !branch_exists(&branch, &project.dir)? {
            // Try to check out from remote if not present locally
            let remote_branch = format!("origin/{}", branch);
            if branch_exists(&remote_branch, &project.dir)? {
                if let Err(e) =
                    run_git_command(&["checkout", "-B", &branch, &remote_branch], &project.dir)
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
                        theme.highlight(branch.as_str()),
                        theme.success("from remote.")
                    );
                }
            } else {
                println!(
                    "  {} {}",
                    theme.error("Branch"),
                    theme.highlight(branch.as_str()),
                );
                println!("    {}", theme.error("not found locally or on remote."));
                has_issue = true;
            }
        } else {
            if let Err(e) = run_git_command(&["checkout", &branch], &project.dir) {
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
                    theme.highlight(branch.as_str())
                );
            }
        }

        // 4. Reset hard to remote branch
        println!(
            "  Resetting to {}...",
            theme.highlight(&format!("origin/{}", branch))
        );
        if let Err(e) = run_git_command(
            &["reset", "--hard", &format!("origin/{}", branch)],
            &project.dir,
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
        if let Err(e) = run_git_command(&["clean", "-fd"], &project.dir) {
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
                "  {} {} {}",
                theme.success("Ready for"),
                theme.highlight("new feature branch."),
                ""
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

// --- Utility functions (adapted from switch.rs) ---

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

fn get_current_branch(dir: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()?;
    if !output.status.success() {
        return Err(eyre::eyre!("Failed to get current branch"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

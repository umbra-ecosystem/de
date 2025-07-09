pub mod switch;

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
        theme.info(&format!(
            "Resetting workspace to base branch '{}'...",
            branch
        ))
    );

    let mut projects_with_issues = Vec::new();
    let mut projects_ready = Vec::new();

    let mut aborted = false;
    'project_loop: for (project_name, project) in workspace.config().projects.iter() {
        if aborted {
            break;
        }
        let mut messages = Vec::new();
        let mut has_issue = false;

        messages.push(theme.info(&format!("  - Project: {}", project_name)));

        // 1. Fetch all remotes
        messages.push(theme.info("    Fetching remotes..."));
        if let Err(e) = run_git_command(&["fetch", "--all", "--prune"], &project.dir) {
            messages.push(theme.error(&format!("    FETCH FAILED: {}", e)));
            has_issue = true;
        }

        // 2. Check for uncommitted changes
        let dirty = is_project_dirty(&project.dir).unwrap_or(false);
        if dirty {
            messages.push(theme.warn("    Uncommitted changes detected!"));
            let mut action = on_dirty;
            if on_dirty == OnDirtyAction::Prompt {
                // Show project context before prompt
                println!(
                    "    {} ({})",
                    theme.info(&format!("Project: {}", project_name)),
                    project.dir.display()
                );
                // Optionally, show current branch
                if let Ok(branch) = get_current_branch(&project.dir) {
                    println!("    Current branch: {}", branch);
                }
                let choices = &[
                    "Stash changes and proceed",
                    "Force reset (discard all changes)",
                    "Skip this project",
                    "Abort all (stop processing)",
                ];
                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("Uncommitted changes detected. What do you want to do?")
                    .default(0)
                    .items(choices)
                    .interact()?;
                match selection {
                    0 => action = OnDirtyAction::Stash,
                    1 => action = OnDirtyAction::Force,
                    2 => {
                        messages.push(theme.warn("    Skipped by user."));
                        for message in messages {
                            println!("{}", message);
                        }
                        continue 'project_loop;
                    }
                    3 => {
                        messages.push(
                            theme.error("    Aborted by user. Stopping all further processing."),
                        );
                        aborted = true;
                        for message in messages {
                            println!("{}", message);
                        }
                        break;
                    }
                    _ => unreachable!(),
                }
                if aborted {
                    break;
                }
            }

            match action {
                OnDirtyAction::Stash => {
                    messages.push(theme.info("    Stashing changes..."));
                    if let Err(e) = run_git_command(&["stash", "push", "-u"], &project.dir) {
                        messages.push(theme.error(&format!("    STASH FAILED: {}", e)));
                        has_issue = true;
                    }
                }
                OnDirtyAction::Force => {
                    messages.push(theme.warn("    Discarding all local changes..."));
                    if let Err(e) = run_git_command(&["reset", "--hard"], &project.dir) {
                        messages.push(theme.error(&format!("    RESET FAILED: {}", e)));
                        has_issue = true;
                    }
                }
                OnDirtyAction::Abort => {
                    messages.push(theme.warn("    Aborting preparation for this project."));
                    for message in messages {
                        println!("{}", message);
                    }
                    projects_with_issues.push(project_name.to_string());
                    continue;
                }
                OnDirtyAction::Prompt => {} // already handled
            }
        } else {
            messages.push(theme.info("    Working directory clean."));
        }

        // 3. Checkout the base branch
        messages.push(theme.info(&format!("    Checking out branch '{}'...", branch)));
        if !branch_exists(&branch, &project.dir)? {
            // Try to check out from remote if not present locally
            let remote_branch = format!("origin/{}", branch);
            if branch_exists(&remote_branch, &project.dir)? {
                if let Err(e) =
                    run_git_command(&["checkout", "-B", &branch, &remote_branch], &project.dir)
                {
                    messages.push(theme.error(&format!("    CHECKOUT FAILED: {}", e)));
                    has_issue = true;
                } else {
                    messages
                        .push(theme.success(&format!("    Checked out '{}' from remote.", branch)));
                }
            } else {
                messages.push(theme.error(&format!(
                    "    Branch '{}' not found locally or on remote.",
                    branch
                )));
                has_issue = true;
            }
        } else {
            if let Err(e) = run_git_command(&["checkout", &branch], &project.dir) {
                messages.push(theme.error(&format!("    CHECKOUT FAILED: {}", e)));
                has_issue = true;
            } else {
                messages.push(theme.success(&format!("    Checked out '{}'.", branch)));
            }
        }

        // 4. Reset hard to remote branch
        messages.push(theme.info(&format!("    Resetting to origin/{}...", branch)));
        if let Err(e) = run_git_command(
            &["reset", "--hard", &format!("origin/{}", branch)],
            &project.dir,
        ) {
            messages.push(theme.error(&format!("    RESET FAILED: {}", e)));
            has_issue = true;
        } else {
            messages.push(theme.success("    Reset complete."));
        }

        // 5. Clean untracked files
        messages.push(theme.info("    Cleaning untracked files..."));
        if let Err(e) = run_git_command(&["clean", "-fd"], &project.dir) {
            messages.push(theme.error(&format!("    CLEAN FAILED: {}", e)));
            has_issue = true;
        } else {
            messages.push(theme.success("    Clean complete."));
        }

        // 6. Final status
        if !has_issue {
            messages.push(theme.success("    Ready for new feature branch."));
            projects_ready.push(project_name.to_string());
        } else {
            projects_with_issues.push(project_name.to_string());
        }

        for message in messages {
            println!("{}", message);
        }
        if aborted {
            break;
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

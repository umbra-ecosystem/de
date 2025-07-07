use std::process::Command;

use dialoguer::{Select, theme::ColorfulTheme};
use eyre::Result;

use crate::{cli::OnDirtyAction, utils::theme::Theme, workspace::Workspace};

pub fn switch(
    target_branch: String,
    fallback: Option<String>,
    on_dirty: Option<OnDirtyAction>,
) -> Result<()> {
    let theme = Theme::new();
    let workspace =
        Workspace::active()?.ok_or_else(|| eyre::eyre!("No active workspace found."))?;

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

    println!("\n{}", theme.info("Synchronization complete."));

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

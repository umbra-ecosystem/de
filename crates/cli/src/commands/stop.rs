use std::path::PathBuf;

use crate::{
    commands::git::{describe_risk, project_dirs},
    config::Config,
    project::Project,
    types::Slug,
    utils::ui::UserInterface,
    workspace::{Workspace, spin_down_workspace},
};
use de_core::git::{RiskReport, assess_risks, status_all};
use dialoguer::Confirm;
use eyre::{Context, eyre};

pub fn stop(workspace_name: Option<Slug>, yes: bool) -> eyre::Result<()> {
    let ui = UserInterface::new();

    if let Some(workspace_name) = workspace_name {
        // Workspace mode - existing logic
        let workspace = Workspace::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

        stop_guarded(&ui, workspace, yes)?;
    } else if let Some(active_workspace) = Workspace::active()? {
        stop_guarded(&ui, active_workspace, yes)?;
    } else {
        // Current project mode - can use inferred project
        let project = Project::current_or_inferred()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found"))?;

        if project.is_inferred() {
            let name = project.manifest().project().name.clone();
            let dirs = [(name, project.dir().clone())];
            if !confirm_stop(&ui, &dirs, yes)? {
                return Ok(());
            }

            // Simple stop for inferred projects
            ui.writeln(
                &ui.theme
                    .bold(&format!("Stopping {}:", project.manifest().project().name)),
            )?;

            let stopped = project
                .docker_compose_down()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to stop docker compose services")?;

            if !stopped {
                ui.warning_item("No docker-compose file found", None)?;
            }
        } else {
            // Existing logic for configured projects with workspace
            let workspace = Workspace::active()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get current workspace")?
                .ok_or_else(|| eyre!("No workspace is currently active"))?;

            stop_guarded(&ui, workspace, yes)?;
        }
    }

    Ok(())
}

/// Stop a workspace after making sure no uncommitted or unpushed work is lost.
fn stop_guarded(ui: &UserInterface, workspace: Workspace, yes: bool) -> eyre::Result<()> {
    if !confirm_stop(ui, &project_dirs(&workspace), yes)? {
        return Ok(());
    }
    stop_workspace(ui, workspace)?;
    Ok(())
}

/// Check the projects for uncommitted or unpushed work. Projects that are not
/// git repositories or cannot be read are reported separately and never count
/// as at risk. Returns data only: nothing here prompts or prints.
pub fn check_git_risks(projects: &[(Slug, PathBuf)]) -> RiskReport<Slug> {
    assess_risks(status_all(projects))
}

/// Show what would be lost and ask before going ahead. `true` means proceed.
fn confirm_stop(ui: &UserInterface, projects: &[(Slug, PathBuf)], yes: bool) -> eyre::Result<bool> {
    let report = check_git_risks(projects);

    for (name, reason) in &report.unreadable {
        ui.warning_item(
            &format!("Could not check the git state of '{name}'"),
            Some(reason.lines().next().unwrap_or(reason)),
        )?;
    }

    if report.is_safe() {
        return Ok(true);
    }

    ui.warning_item(
        "These projects have work that spinning down does not preserve:",
        None,
    )?;
    ui.indented(|ui| {
        for risk in &report.at_risk {
            ui.info_item(&format!("{}: {}", risk.name, describe_risk(&risk.status)))?;
        }
        Ok(())
    })?;

    if yes {
        return Ok(true);
    }

    let proceed = Confirm::new()
        .with_prompt("Stop anyway?")
        .default(false)
        .interact()
        .wrap_err("Could not ask for confirmation (use --yes to skip)")?;
    if !proceed {
        ui.writeln("Aborted.")?;
    }
    Ok(proceed)
}

pub fn stop_workspace(_ui: &UserInterface, workspace: Workspace) -> eyre::Result<bool> {
    spin_down_workspace(&workspace)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to spin down workspace")?;

    deactivate_workspace_if_active(workspace.config().name.clone())
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to deactivate workspace in config")?;

    Ok(true)
}

fn deactivate_workspace_if_active(workspace_name: Slug) -> eyre::Result<()> {
    let mut config = Config::load()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load application config")?;

    let Some(active_workspace_name) = config.get_active_workspace() else {
        return Ok(());
    };

    if active_workspace_name == &workspace_name {
        config.set_active_workspace(None);
        config
            .save()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to save application config")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{path::Path, process::Command};

    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-b", "main"]);
        run(dir.path(), &["config", "user.name", "Test"]);
        run(dir.path(), &["config", "user.email", "t@example.com"]);
        run(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        run(dir.path(), &["add", "-A"]);
        run(dir.path(), &["commit", "-m", "initial"]);
        // Pretend the commit is on a remote so only the working tree matters.
        run(
            dir.path(),
            &["update-ref", "refs/remotes/origin/main", "HEAD"],
        );
        dir
    }

    fn slug(name: &str) -> Slug {
        name.parse().unwrap()
    }

    #[test]
    fn guard_reports_dirty_projects_and_only_warns_about_unreadable_ones() {
        let clean = repo_with_commit();
        let dirty = repo_with_commit();
        std::fs::write(dirty.path().join("a.txt"), "edited\n").unwrap();
        let not_git = tempfile::tempdir().unwrap();

        let projects = vec![
            (slug("clean"), clean.path().to_path_buf()),
            (slug("dirty"), dirty.path().to_path_buf()),
            (slug("plain"), not_git.path().to_path_buf()),
            (slug("gone"), PathBuf::from("/definitely/not/a/dir")),
        ];
        let report = check_git_risks(&projects);

        assert!(!report.is_safe());
        assert_eq!(report.at_risk.len(), 1);
        assert_eq!(report.at_risk[0].name.as_str(), "dirty");
        assert_eq!(report.at_risk[0].status.modified, 1);
        assert_eq!(report.unreadable.len(), 2);

        // Unreadable projects alone never block stopping.
        let only_bad = check_git_risks(&projects[2..]);
        assert!(only_bad.is_safe());
        assert_eq!(only_bad.unreadable.len(), 2);
    }
}

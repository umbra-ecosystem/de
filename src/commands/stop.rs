use crate::{
    commands::status::workspace_status,
    config::Config,
    project::Project,
    types::Slug,
    utils::ui::UserInterface,
    workspace::{Workspace, spin_down_workspace},
};
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

        stop_workspace(&ui, workspace, yes)?;
    } else {
        // Current project mode - can use inferred project
        let project = Project::current_or_inferred()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found"))?;

        if project.is_inferred() {
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

            stop_workspace(&ui, workspace, yes)?;
        }
    }

    Ok(())
}

pub fn stop_workspace(ui: &UserInterface, workspace: Workspace, yes: bool) -> eyre::Result<bool> {
    let workspace_status = workspace_status(ui, &workspace)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get workspace status")?;

    ui.new_line()?;

    if workspace_status.has_uncommited_or_unpushed_changes() && !yes {
        let prompt = Confirm::new()
            .with_prompt("Uncommitted or unpushed changes detected. Stop anyway?")
            .default(false)
            .interact()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to prompt for confirmation")?;

        if !prompt {
            println!("Aborting stop operation.");
            return Ok(false);
        }
    }

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

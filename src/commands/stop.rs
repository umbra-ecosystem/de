use crate::{
    commands::status::workspace_status,
    config::Config,
    types::Slug,
    workspace::{Workspace, spin_down_workspace},
    utils::formatter::Formatter,
};
use dialoguer::Confirm;
use eyre::{Context, eyre};

pub fn stop(workspace_name: Option<Slug>) -> eyre::Result<()> {
    let workspace = if let Some(workspace_name) = workspace_name {
        Workspace::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?
    } else {
        Workspace::active()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current workspace")?
            .ok_or_else(|| eyre!("No workspace is currently active"))?
    };

    let workspace_status = {
        let formatter = Formatter::new();
        workspace_status(&workspace, &formatter)
    }
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get workspace status")?;

    println!();

    if workspace_status.has_uncommited_or_unpushed_changes() {
        let prompt = Confirm::new()
            .with_prompt("Uncommitted or unpushed changes detected. Stop anyway?")
            .default(false)
            .interact()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to prompt for confirmation")?;

        if !prompt {
            println!("Aborting stop operation.");
            return Ok(());
        }
    }

    spin_down_workspace(&workspace)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to spin down workspace")?;

    deactivate_workspace_if_active(workspace.config().name.clone())
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to deactivate workspace in config")?;

    Ok(())
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

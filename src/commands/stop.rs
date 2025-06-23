use crate::{
    config::Config,
    types::Slug,
    workspace::{Workspace, spin_down_workspace},
};
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

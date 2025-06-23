use eyre::{WrapErr, eyre};

use crate::{
    config::Config,
    project::Project,
    types::Slug,
    workspace::{Workspace, spin_up_workspace},
};

pub fn start(workspace_name: Option<Slug>) -> eyre::Result<()> {
    let workspace_name = if let Some(name) = workspace_name {
        name
    } else {
        let project = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found"))?;

        project.manifest().workspace().name.clone()
    };

    let workspace = Workspace::load_from_name(&workspace_name)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load workspace")?
        .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

    spin_up_workspace(&workspace)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to spin up workspace")?;

    Config::mutate_persisted(|config| {
        config.set_active_workspace(Some(workspace_name));
    })?;

    Ok(())
}

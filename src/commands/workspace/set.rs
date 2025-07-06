use eyre::{Context, eyre};

use crate::{config::Config, types::Slug, utils::theme::Theme, workspace::Workspace};

pub fn set(workspace_name: Slug) -> eyre::Result<()> {
    let workspace = Workspace::load_from_name(&workspace_name)
        .wrap_err("Failed to load workspace")?
        .ok_or_else(|| eyre!("Workspace '{}' not found", workspace_name))?;

    Config::mutate_persisted(|config| {
        config.set_active_workspace(Some(workspace.config().name.clone()));
    })?;

    let theme = Theme::new();
    println!(
        "Switched to workspace: {}",
        theme.info(workspace.config().name.as_str())
    );

    Ok(())
}

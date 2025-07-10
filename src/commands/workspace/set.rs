use eyre::{Context, eyre};

use crate::{config::Config, types::Slug, utils::theme::Theme, workspace::Workspace};

/// Set or get a property on the workspace (e.g., active, default-branch).
pub fn set(workspace_name: Option<Slug>, key: String, value: Option<String>) -> eyre::Result<()> {
    let mut workspace = if let Some(name) = workspace_name {
        Workspace::load_from_name(&name)
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", name))?
    } else {
        Workspace::active()
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No active workspace found"))?
    };

    match key.as_str() {
        "active" => {
            if let Some(_) = value {
                Config::mutate_persisted(|config| {
                    config.set_active_workspace(Some(workspace.config().name.clone()));
                })?;
                let theme = Theme::new();
                println!(
                    "Switched to workspace: {}",
                    theme.highlight(workspace.config().name.as_str())
                );
            } else {
                // Print if this workspace is active
                let config = Config::load()?;
                let is_active = config
                    .get_active_workspace()
                    .map(|n| n == &workspace.config().name)
                    .unwrap_or(false);
                println!(
                    "Workspace '{}' is {}active.",
                    workspace.config().name,
                    if is_active { "" } else { "not " }
                );
            }
        }
        "default-branch" | "default_branch" => {
            if let Some(branch) = value {
                workspace.config_mut().default_branch = Some(branch.clone());
                workspace
                    .save()
                    .wrap_err("Failed to save workspace configuration")?;
                println!(
                    "Default branch for workspace '{}' set to '{}'.",
                    workspace.config().name,
                    branch
                );
            } else {
                match &workspace.config().default_branch {
                    Some(branch) => println!("{}", branch),
                    None => println!(
                        "No default branch set for workspace '{}'.",
                        workspace.config().name
                    ),
                }
            }
        }
        _ => {
            return Err(eyre!("Unknown property key '{}'", key));
        }
    }

    Ok(())
}

/// Unset a property on the workspace.
pub fn unset(workspace_name: Option<Slug>, key: String) -> eyre::Result<()> {
    let mut workspace = if let Some(name) = workspace_name {
        Workspace::load_from_name(&name)
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", name))?
    } else {
        Workspace::active()
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No active workspace found"))?
    };

    match key.as_str() {
        "active" => {
            Config::mutate_persisted(|config| {
                config.set_active_workspace(None);
            })
            .wrap_err("Failed to unset active workspace")?;
            println!("Active workspace unset.");
        }
        "default-branch" | "default_branch" => {
            workspace.config_mut().default_branch = None;
            workspace
                .save()
                .wrap_err("Failed to save workspace configuration")?;
            println!(
                "Default branch for workspace '{}' has been unset.",
                workspace.config().name
            );
        }
        _ => {
            return Err(eyre!("Unknown property key '{}'", key));
        }
    }

    Ok(())
}

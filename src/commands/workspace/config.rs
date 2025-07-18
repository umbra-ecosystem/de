
use eyre::{Context, eyre};

use crate::{
    commands::config::ConfigAction, types::Slug,
    workspace::Workspace,
};

/// Set or get a property on the workspace (e.g., active, default-branch).
pub fn config(
    workspace_name: Option<Slug>,
    key: String,
    value: Option<String>,
    unset: bool,
) -> eyre::Result<()> {
    let mut workspace = if let Some(name) = workspace_name {
        Workspace::load_from_name(&name)
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", name))?
    } else {
        Workspace::active()
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No active workspace found"))?
    };

    let action = if unset {
        ConfigAction::Unset
    } else if let Some(value) = value {
        ConfigAction::Set(value)
    } else {
        ConfigAction::Show
    };

    match key.as_str() {
        "default-branch" | "default_branch" => match action {
            ConfigAction::Show => match &workspace.config().default_branch {
                Some(branch) => println!("{branch}"),
                None => println!(
                    "No default branch set for workspace '{}'.",
                    workspace.config().name
                ),
            },
            ConfigAction::Set(branch) => {
                workspace.config_mut().default_branch = Some(branch.clone());
                workspace
                    .save()
                    .wrap_err("Failed to save workspace configuration")?;
                println!(
                    "Default branch for workspace '{}' set to '{}'.",
                    workspace.config().name,
                    branch
                );
            }
            ConfigAction::Unset => {
                workspace.config_mut().default_branch = None;
                workspace
                    .save()
                    .wrap_err("Failed to save workspace configuration")?;
                println!(
                    "Default branch remove from workspace '{}'",
                    workspace.config().name,
                );
            }
        },
        _ => {
            return Err(eyre!("Unknown property key '{}'", key));
        }
    }

    Ok(())
}

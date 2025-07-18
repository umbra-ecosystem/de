use std::str::FromStr;

use eyre::{Context, eyre};

use crate::{config::Config, types::Slug, utils::theme::Theme, workspace::Workspace};

enum Action {
    Show,
    Set(String),
    Unset,
}

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
        Action::Unset
    } else if let Some(value) = value {
        Action::Set(value)
    } else {
        Action::Show
    };

    match key.as_str() {
        "active" => match action {
            Action::Show => {
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
            Action::Set(value) => {
                let workspace_name = Slug::from_str(&value)
                    .map_err(|e| eyre!(e))
                    .wrap_err("Invalid workspace name")?;

                Config::mutate_persisted(|config| {
                    config.set_active_workspace(Some(workspace_name));
                })?;

                let theme = Theme::new();
                println!(
                    "Switched to workspace: {}",
                    theme.highlight(workspace.config().name.as_str())
                );
            }
            Action::Unset => {
                Config::mutate_persisted(|config| {
                    config.set_active_workspace(None);
                })?;

                let theme = Theme::new();
                println!(
                    "Switched to workspace: {}",
                    theme.highlight(workspace.config().name.as_str())
                );
            }
        },
        "default-branch" | "default_branch" => match action {
            Action::Show => match &workspace.config().default_branch {
                Some(branch) => println!("{}", branch),
                None => println!(
                    "No default branch set for workspace '{}'.",
                    workspace.config().name
                ),
            },
            Action::Set(branch) => {
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
            Action::Unset => {
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

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{config::Config, types::Slug, utils::theme::Theme, workspace::registry};
use eyre::{WrapErr, eyre};

pub enum ConfigAction {
    Show,
    Set(String),
    Unset,
}

pub fn config(key: String, value: Option<String>, unset: bool) -> eyre::Result<()> {
    let action = if unset {
        ConfigAction::Unset
    } else if let Some(value) = value {
        ConfigAction::Set(value)
    } else {
        ConfigAction::Show
    };

    match key.as_str() {
        "active" => match action {
            ConfigAction::Show => {
                let current_config = Config::load()
                    .map_err(|e| eyre!(e))
                    .wrap_err("Failed to load application config")?;

                match current_config.get_active_workspace() {
                    Some(workspace_name) => {
                        let theme = Theme::new();
                        println!(
                            "Active workspace: {}",
                            theme.highlight(workspace_name.as_str())
                        );
                    }
                    None => println!("No active workspace set."),
                }
            }
            ConfigAction::Set(value) => {
                let workspace_name = Slug::from_str(&value)
                    .map_err(|e| eyre!(e))
                    .wrap_err("Invalid workspace name")?;

                // Validated and recorded the same way `de workspace select` does it, so a
                // name that is not saved never ends up in config.toml.
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                registry::select_workspace(&workspace_name, now).map_err(|e| eyre!(e))?;

                let theme = Theme::new();
                println!(
                    "Switched to workspace: {}",
                    theme.highlight(workspace_name.as_str())
                );
            }
            ConfigAction::Unset => {
                registry::deselect_workspace()
                    .map_err(|e| eyre!(e))?;

                println!("Unset active workspace.");
            }
        },
        _ => {
            return Err(eyre!(
                "Unknown configuration key: '{}'. Supported keys: active",
                key
            ));
        }
    }

    Ok(())
}

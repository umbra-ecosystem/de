use std::str::FromStr;

use crate::{config::Config, types::Slug, utils::theme::Theme};
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

                Config::mutate_persisted(|config| {
                    config.set_active_workspace(Some(workspace_name.clone()));
                })?;

                let theme = Theme::new();
                println!(
                    "Switched to workspace: {}",
                    theme.highlight(workspace_name.as_str())
                );
            }
            ConfigAction::Unset => {
                Config::mutate_persisted(|config| {
                    config.set_active_workspace(None);
                })?;

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

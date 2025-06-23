use eyre::{Context, eyre};
use serde::{Deserialize, Serialize};

use crate::{types::Slug, utils::get_project_dirs};

/// Global configuration for the application.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// The active workspace configuration.
    pub active: Option<ActiveConfig>,
}

/// Configuration for the active workspace.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ActiveConfig {
    pub workspace: Option<Slug>,
}

impl Config {
    pub fn config_path() -> eyre::Result<std::path::PathBuf> {
        let project_dirs = get_project_dirs()?;
        Ok(project_dirs.config_dir().join("config.toml"))
    }

    pub fn save(&self) -> eyre::Result<()> {
        let config_path = Self::config_path()?;

        let config_str = toml::to_string_pretty(self)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to format config as string")?;

        std::fs::write(&config_path, config_str)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to write config to {}", config_path.display()))?;

        Ok(())
    }

    pub fn load() -> eyre::Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!("Failed to read config file at {}", config_path.display())
                })?;
            toml::from_str(&config_str)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to parse config file")
        } else {
            Ok(Self::default())
        }
    }
}

impl Config {
    /// Loads the current configuration, applies the provided mutation function, and saves the modified configuration.
    pub fn mutate_persisted<F>(f: F) -> eyre::Result<Config>
    where
        F: FnOnce(&mut Config),
    {
        let mut config = Self::load()?;
        f(&mut config);
        config
            .save()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to save modified workspace configuration")?;
        Ok(config)
    }
}

impl Config {
    pub fn get_active_workspace(&self) -> Option<&Slug> {
        self.active
            .as_ref()
            .and_then(|active| active.workspace.as_ref())
    }

    pub fn set_active_workspace(&mut self, workspace: Option<Slug>) {
        if let Some(active) = &mut self.active {
            active.workspace = workspace;
        } else {
            self.active = Some(ActiveConfig { workspace });
        }
    }
}

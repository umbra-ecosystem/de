pub mod cli;
pub mod formatter;
pub mod git;
pub mod serde;
pub mod shim;
pub mod theme;
pub mod ui;
pub mod unix;
pub mod zip;

use crate::constants::{ORGANIZATION_NAME, PROJECT_NAME};

pub use cli::{get_project_for_cli, get_workspace_for_cli};
pub use shim::{check_shim_installation_in_shell_config, get_shims_dir, shim_export_line};

pub fn get_project_dirs() -> eyre::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", ORGANIZATION_NAME, PROJECT_NAME)
        .ok_or_else(|| eyre::eyre!("Failed to get project directories"))
}

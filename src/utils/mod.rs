mod shim;
pub mod unix;

use crate::constants::{ORGANIZATION_NAME, PROJECT_NAME};

pub use shim::{
    check_shim_installation_in_shell_config, generate_shim_bash_script, get_shims_dir,
    shim_export_line,
};

pub fn get_project_dirs() -> eyre::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", ORGANIZATION_NAME, PROJECT_NAME)
        .ok_or_else(|| eyre::eyre!("Failed to get project directories"))
}

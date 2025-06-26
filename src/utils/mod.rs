mod shim;

use crate::constants::{ORGANIZATION_NAME, PROJECT_NAME};

pub use shim::{generate_shim_bash_script, get_shims_dir};

pub fn get_project_dirs() -> eyre::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("", ORGANIZATION_NAME, PROJECT_NAME)
        .ok_or_else(|| eyre::eyre!("Failed to get project directories"))
}

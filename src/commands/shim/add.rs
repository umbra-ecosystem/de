use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use crate::{
    types::Slug,
    utils::{generate_shim_bash_script, get_shims_dir},
};
use eyre::{WrapErr, eyre};

pub fn add(command: Slug) -> eyre::Result<()> {
    let shims_dir = get_shims_dir()?;
    let shim_path = shims_dir.join(format!("{}", command));

    let shim_program = generate_shim_bash_script(command.as_str());
    std::fs::create_dir_all(&shims_dir)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to create shims directory: {}", shims_dir.display()))?;

    std::fs::write(&shim_path, shim_program)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write shim to {}", shim_path.display()))?;

    apply_executable_permissions(&shim_path)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to set executable permissions for {}",
                shim_path.display()
            )
        })?;

    Ok(())
}

fn apply_executable_permissions(shim_file: &Path) -> eyre::Result<()> {
    let mut permissions = fs::metadata(shim_file)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111); // Add execute permissions for owner, group, others

    fs::set_permissions(shim_file, permissions)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to set permissions for {}", shim_file.display()))?;

    Ok(())
}

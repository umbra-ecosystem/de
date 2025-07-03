use eyre::{Context, eyre};

use crate::utils::{
    check_shim_installation_in_shell_config, get_shims_dir, shim_export_line,
    unix::{get_shell_config_paths, primary_shell_config_path},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn install() -> eyre::Result<()> {
    let shims_dir = get_shims_dir()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get shims directory")?;

    let shell_config_paths = get_shell_config_paths()?;

    let is_installed = check_shim_installation(&shell_config_paths, &shims_dir)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to check shim installation")?;

    if is_installed {
        println!("Shim is already installed in your shell configuration.");
        return Ok(());
    }

    // If not installed, add the shims directory to the shell configuration files
    let file = primary_shell_config_path()?;

    // Ensure the shims directory exists before adding it to the shell config
    if !shims_dir.exists() {
        fs::create_dir_all(&shims_dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to create shims directory: {}", shims_dir.display())
            })?;
    }

    add_to_shell_config(&file, &shims_dir)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to add shim to shell config: {}", file.display()))?;

    Ok(())
}

/// Check if the shim is installed in the user's shell configuration files
fn check_shim_installation(config_files: &[PathBuf], install_dir: &Path) -> eyre::Result<bool> {
    for config_file in config_files {
        if !config_file.exists() {
            // If the config file does not exist, we cannot check for the shim installation
            continue;
        }

        let is_installed = check_shim_installation_in_shell_config(config_file, install_dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!(
                    "Failed to check shim installation in shell config: {}",
                    config_file.display()
                )
            })?;

        if is_installed {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Add the installation directory to the user's shell configuration file
fn add_to_shell_config(config_file_path: &Path, install_dir: &Path) -> eyre::Result<()> {
    let shim_export = shim_export_line(install_dir)?;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config_file_path)?;

    // Append a newline before the new line for better formatting, then the export line
    file.write_all(b"\n")?;
    file.write_all(shim_export.as_bytes())?;
    file.write_all(b"\n")?; // Another newline after for cleanliness

    Ok(())
}

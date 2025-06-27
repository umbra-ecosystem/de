use std::path::Path;

use eyre::{WrapErr, eyre};

use crate::utils::{
    check_shim_installation_in_shell_config, get_shims_dir, shim_export_line,
    unix::get_shell_config_paths,
};

pub fn uninstall() -> eyre::Result<()> {
    let shims_dir = get_shims_dir()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get shims directory")?;

    let shell_config_paths = get_shell_config_paths()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get shell configuration paths")?;

    for file in shell_config_paths {
        if check_shim_installation_in_shell_config(&file, &shims_dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to check shim installation in {}", file.display()))?
        {
            remove_from_shell_config(&file, &shims_dir)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to remove shim from shell config: {}",
                        file.display()
                    )
                })?;
            println!("Removed shim from shell configuration: {}", file.display());
        } else {
            tracing::info!("No shim found in shell configuration: {}", file.display());
        }
    }

    Ok(())
}

fn remove_from_shell_config(file: &Path, shims_dir: &Path) -> eyre::Result<()> {
    if !file.exists() {
        return Ok(());
    }

    let shim_export = shim_export_line(shims_dir)?;

    let content = std::fs::read_to_string(file)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to read file: {}", file.display()))?;

    let new_content = content
        .lines()
        .filter(|line| !line.contains(&shim_export))
        .collect::<Vec<&str>>()
        .join("\n");

    std::fs::write(file, new_content)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write to file: {}", file.display()))?;

    Ok(())
}

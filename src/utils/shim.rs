use eyre::{WrapErr, eyre};
use std::path::Path;

use crate::{types::Slug, utils::get_project_dirs};

pub fn get_shims_dir() -> eyre::Result<std::path::PathBuf> {
    let dirs = get_project_dirs()?;
    Ok(dirs.data_dir().join("shims"))
}

pub fn shim_export_line(shims_dir: &Path) -> eyre::Result<String> {
    let shims_dir_str = shims_dir
        .to_str()
        .ok_or_else(|| eyre::eyre!("Failed to convert shims directory path to string"))?;
    Ok(format!("export PATH=\"{shims_dir_str}:$PATH\""))
}

pub fn check_shim_installation_in_shell_config(
    file: &Path,
    shims_dir: &Path,
) -> eyre::Result<bool> {
    if !file.exists() {
        return Ok(false);
    }

    // This is above content read to avoid reading the file if export line fails
    let shim_export = shim_export_line(shims_dir)?;

    let content = std::fs::read_to_string(file)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to read file: {}", file.display()))?;

    Ok(content.contains(&shim_export))
}

pub fn write_shim_to_file(command: &Slug) -> eyre::Result<()> {
    let shims_dir = get_shims_dir()?;
    let shim_file = shims_dir.join(format!("{command}"));

    let shim_program = generate_shim_bash_script(command.as_str());
    std::fs::create_dir_all(&shims_dir)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to create shims directory: {}", shims_dir.display()))?;

    std::fs::write(&shim_file, shim_program)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write shim to {}", shim_file.display()))?;

    #[cfg(target_family = "unix")]
    apply_executable_permissions(&shim_file)?;

    Ok(())
}

#[cfg(target_family = "unix")]
fn apply_executable_permissions(shim_file: &Path) -> eyre::Result<()> {
    use std::{fs, os::unix::fs::PermissionsExt};

    let mut permissions = fs::metadata(shim_file)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111); // Add execute permissions for owner, group, others

    fs::set_permissions(shim_file, permissions)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to set permissions for {}", shim_file.display()))?;

    Ok(())
}

pub fn get_installed_shims() -> eyre::Result<Vec<String>> {
    let shims_dir = get_shims_dir()?;

    if !shims_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(&shims_dir)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to read shims directory: {}", shims_dir.display()))?;

    let mut shims = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| eyre!(e)).wrap_err_with(|| {
            format!(
                "Failed to read entry in shims directory: {}",
                shims_dir.display()
            )
        })?;

        if let Some(name) = entry.file_name().to_str() {
            shims.push(name.to_string());
        }
    }

    Ok(shims)
}

pub fn generate_shim_bash_script(program_name: &str) -> String {
    format!(
        r##"#!/bin/bash
# This script is auto-generated and should not be manually edited.

# This script wraps the '{program_name}' command.
# It prioritizes 'de run {program_name}' if 'de task check {program_name}' passes silently.
# Otherwise, it falls back to the system's original '{program_name}' command.

# Executes the Nth occurrence of a command found in PATH.
# Essential for shims to call the original command without infinite recursion.
# Args: $1=command_name, $2=occurrence_number (defaults to 1), $@=command_arguments.
exec_nth_command() {{
    local command_name="$1"
    local n="${{2:-1}}"
    local path_found=""
    local current_match_count=0

    # Validate inputs.
    [[ -z "$command_name" ]] && {{ echo "Error: Command name missing." >&2; return 1; }}
    [[ ! "$n" =~ ^[1-9][0-9]*$ ]] && {{ echo "Error: 'n' must be a positive integer." >&2; return 1; }}

    # Search PATH for the Nth executable.
    IFS=':' read -ra path_dirs <<< "$PATH"
    for dir in "${{path_dirs[@]}}"; do
        local full_path="$dir/$command_name"
        if [[ -x "$full_path" && ! -d "$full_path" ]]; then
            current_match_count=$((current_match_count + 1))
            [[ "$current_match_count" -eq "$n" ]] && {{ path_found="$full_path"; break; }}
        fi
    done

    # Execute or error. 'exec' replaces current process.
    if [[ -n "$path_found" ]]; then
        exec "$path_found" "${{@:3}}"
    else
        echo "Error: ${{n}}th occurrence of '$command_name' not found in PATH." >&2
        return 1
    fi
}}

# --- Main Logic ---

# Silently check 'de task {program_name}'. Redirects all output to /dev/null.
if de task check {program_name} >/dev/null 2>&1; then
    # If check passes, execute 'de run {program_name}'. 'exec' avoids new process.
    exec de run {program_name} -- "$@"
else
    # If check fails or 'de' not found, fall back to system command.
    # Calls the 2nd instance of '{program_name}' in PATH (assuming 1st is this shim).
    exec_nth_command "{program_name}" 2 "$@"
fi
"##
    )
}

use crate::utils::get_project_dirs;

pub fn get_shims_dir() -> eyre::Result<std::path::PathBuf> {
    let dirs = get_project_dirs()?;
    Ok(dirs.data_dir().join("shims"))
}

pub fn generate_shim_bash_script(program_name: &str) -> String {
    format!(
        r##"#!/bin/bash
# This script wraps the '{program_name}' command.
# It checks 'de task {program_name} check' (output hidden).
# If successful, it runs 'de run {program_name}' with all arguments.
# Otherwise, it falls back to the system's '{program_name}' command.

# This function executes the Nth command found in the PATH.
exec_nth_command() {{
    local command_name="$1"
    local n="${{2:-1}}"
    local path_found=""
    local current_match_count=0

    # Validate command name presence
    [[ -z "$command_name" ]] && return 1

    # Validate 'n' is a positive integer
    [[ ! "$n" =~ ^[1-9][0-9]*$ ]] && return 1

    # Split PATH into an array of directories
    IFS=':' read -ra path_dirs <<< "$PATH"

    # Iterate through each directory in PATH
    for dir in "${{path_dirs[@]}}"; do
        local full_path="$dir/$command_name"

        # Check if the file exists and is executable (and not a directory)
        if [[ -x "$full_path" && ! -d "$full_path" ]]; then
            current_match_count=$((current_match_count + 1))

            # If this is the Nth match, store it and break
            if [[ "$current_match_count" -eq "$n" ]]; then
                path_found="$full_path"
                break
            fi
        fi
    done

    # If the Nth command was found, execute it; otherwise, return an error
    [[ -z "$path_found" ]] && return 1 || exec "$path_found" "${{@:3}}"
}}

# Check 'de task check {program_name}' silently.
# '> /dev/null 2>&1' hides all output.
# '&&' proceeds only if the check is successful.
if de task {program_name} check >/dev/null 2>&1; then
  # If check passes, execute 'de run {program_name}' with all arguments.
  # 'exec' replaces the current process with 'de run {program_name}'.
  exec de run {program_name} "$@"
else
  # If check fails, fall back to the original '{program_name}' command.
  exec exec_nth_command "{program_name}" 2 "$@"
fi
"##
    )
}

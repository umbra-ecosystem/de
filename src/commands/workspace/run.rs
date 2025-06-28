use eyre::{Context, Result, bail, eyre};
use std::process::Command;

use crate::{types::Slug, workspace::Workspace};

pub fn run(workspace_name: Option<Slug>, task_name: Slug, args: Vec<String>) -> Result<()> {
    let workspace = if let Some(workspace_name) = workspace_name {
        Workspace::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace '{}' not found", workspace_name))?
    } else {
        Workspace::active()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No current workspace found"))?
    };

    let task_command = workspace.config().tasks.get(&task_name).ok_or_else(|| {
        eyre!(
            "Task '{}' not found in workspace '{}'",
            task_name,
            workspace.config().name
        )
    })?;

    let mut command_parts = shell_words::split(task_command)
        .map_err(|e| eyre!("Failed to parse task command: {}", e))?;

    let program = command_parts.remove(0);
    let mut task_args = command_parts;
    task_args.extend(args);

    let mut cmd = Command::new(&program);
    cmd.args(&task_args);
    cmd.current_dir(
        workspace
            .config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("/")),
    ); // Run from workspace config directory

    let status = cmd.status()?;
    if !status.success() {
        bail!("Command exited with non-zero status: {}", status);
    }

    Ok(())
}

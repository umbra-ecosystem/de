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

    let mut parts = task_command.split_whitespace();
    let program = parts.next().ok_or_else(|| eyre!("Empty command"))?;
    let task_args = parts.collect::<Vec<_>>();

    let mut cmd = Command::new(&program);
    cmd.args(&task_args);
    cmd.args(&args);

    let status = cmd.status()?;
    if !status.success() {
        bail!("Command exited with non-zero status: {}", status);
    }

    Ok(())
}

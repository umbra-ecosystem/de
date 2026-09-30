use eyre::{Context, Result, eyre};

use crate::{project::shell_command, types::Slug, workspace::Workspace};

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

    let dir = std::env::current_dir().wrap_err("Failed to get current directory")?;
    crate::commands::run::run_to_completion(shell_command(task_command, &dir, &args))
}

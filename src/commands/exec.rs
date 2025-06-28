use eyre::{Context, Result, bail};
use std::process::Command;

use crate::{types::Slug, workspace::Workspace};
use eyre::eyre;

pub fn exec(project_name: Slug, workspace_name: Option<Slug>, command: Vec<String>) -> Result<()> {
    let mut command = command.into_iter();
    let program = command.next().ok_or_else(|| eyre!("No command provided"))?;
    let args = command.collect::<Vec<_>>();

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

    let project = workspace
        .config()
        .projects
        .get(&project_name)
        .ok_or_else(|| {
            eyre!(
                "Project '{}' not found in workspace '{}'",
                project_name,
                workspace.config().name
            )
        })?;

    let mut cmd = Command::new(&program);
    cmd.args(&args);
    cmd.current_dir(&project.dir);

    let status = cmd.status()?;
    if !status.success() {
        bail!("Command exited with non-zero status: {}", status);
    }

    Ok(())
}

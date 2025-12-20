use eyre::{Context, Result, bail};
use std::process::Command;

use crate::{project::Project, types::Slug, utils::ui::UserInterface, workspace::Workspace};
use eyre::eyre;

pub fn exec(
    project_name: Option<Slug>,
    workspace_name: Option<Slug>,
    command: Vec<String>,
) -> Result<()> {
    let ui = UserInterface::new();
    let mut command = command.into_iter();
    let program = command.next().ok_or_else(|| eyre!("No command provided"))?;
    let args = command.collect::<Vec<_>>();

    // If project_name is provided, use workspace mode
    if let Some(project_name) = project_name {
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

        let ws_project = workspace
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
        cmd.current_dir(&ws_project.dir);

        let status = cmd.status()?;
        if !status.success() {
            bail!("Command exited with non-zero status: {}", status);
        }

        return Ok(());
    }

    // No project name provided - use current/inferred project (no-config mode)
    let project = Project::current_or_inferred()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to detect project")?
        .ok_or_else(|| eyre!("No project found in current directory"))?;

    if project.is_inferred() {
        ui.info_item("Running in no-config mode (no de.toml found)")?;
    }

    let project_dir = project.dir();

    let mut cmd = Command::new(&program);
    cmd.args(&args);
    cmd.current_dir(project_dir);

    let status = cmd.status()?;
    if !status.success() {
        bail!("Command exited with non-zero status: {}", status);
    }

    Ok(())
}

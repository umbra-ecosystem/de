use eyre::{Context, Result, eyre};
use std::process::Command;

use crate::{types::Slug, workspace::Workspace};

pub fn exec_all(workspace_name: Option<Slug>, command: Vec<String>) -> Result<()> {
    let mut command_iter = command.into_iter();
    let program = command_iter
        .next()
        .ok_or_else(|| eyre!("No command provided"))?;
    let args = command_iter.collect::<Vec<_>>();

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

    for (project_name, project) in workspace.config().projects.iter() {
        println!("Executing command in project: {project_name}");
        let mut cmd = Command::new(&program);
        cmd.args(&args);
        cmd.current_dir(&project.dir);

        let status = cmd
            .status()
            .wrap_err_with(|| format!("Failed to execute command in project '{project_name}'"))?;
        if !status.success() {
            eprintln!("Command failed in project '{project_name}' with status: {status}");
        }
    }

    Ok(())
}

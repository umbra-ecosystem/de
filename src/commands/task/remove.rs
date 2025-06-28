use eyre::{eyre, Result, Context};

use crate::{project::Project, types::Slug, workspace::Workspace};

pub fn remove(task_name: Slug, is_workspace_task: bool) -> Result<()> {
    if is_workspace_task {
        let mut workspace = Workspace::active()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No active workspace found. Cannot remove workspace task."))?;

        if workspace.config_mut().tasks.remove(&task_name).is_some() {
            workspace.save().wrap_err("Failed to save workspace configuration")?;
            println!("Task '{}' removed from workspace '{}'.", task_name, workspace.config().name);
        } else {
            println!("Task '{}' not found in workspace '{}'.", task_name, workspace.config().name);
        }
    } else {
        let mut project = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found. Cannot remove project task."))?;

        if project.manifest_mut().tasks.get_or_insert_with(Default::default).remove(&task_name).is_some() {
            project.manifest().save(project.manifest_path()).wrap_err("Failed to save project configuration")?;
            println!("Task '{}' removed from project '{}'.", task_name, project.manifest().project().name);
        } else {
            println!("Task '{}' not found in project '{}'.", task_name, project.manifest().project().name);
        }
    }

    Ok(())
}

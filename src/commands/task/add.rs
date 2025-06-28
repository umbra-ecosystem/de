use eyre::{eyre, Result, Context};

use crate::{project::{Project, Task, RawTask}, types::Slug, workspace::Workspace};

pub fn add(task_name: Slug, command: String, service: Option<String>, is_workspace_task: bool) -> Result<()> {
    if is_workspace_task {
        let mut workspace = Workspace::active()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get active workspace")?
            .ok_or_else(|| eyre!("No active workspace found. Cannot add workspace task."))?;

        if service.is_some() {
            return Err(eyre!("Workspace tasks do not support specifying a service."));
        }

        workspace.config_mut().tasks.insert(task_name.clone(), command);
        workspace.save().wrap_err("Failed to save workspace configuration")?;

        println!("Task '{}' added to workspace '{}'.", task_name, workspace.config().name);
    } else {
        let mut project = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found. Cannot add project task."))?;

        let task = if let Some(svc) = service {
            Task::Compose { service: svc, command }
        } else {
            Task::Raw(RawTask::Flat(command))
        };

        project.manifest_mut().tasks.get_or_insert_with(Default::default).insert(task_name.clone(), task);
        project.manifest().save(project.manifest_path()).wrap_err("Failed to save project configuration")?;

        println!("Task '{}' added to project '{}'.", task_name, project.manifest().project().name);
    }

    Ok(())
}

use eyre::{Context, eyre};

use crate::{
    project::{RawTask, Task},
    types::Slug,
    utils::{get_project_for_cli, get_workspace_for_cli},
};

pub fn add(
    task_name: Slug,
    command: String,
    service: Option<String>,
    project_name: Option<Slug>,
    workspace_name: Option<Option<Slug>>,
) -> eyre::Result<()> {
    if workspace_name.is_some() {
        let mut workspace = get_workspace_for_cli(workspace_name)?;

        if service.is_some() {
            return Err(eyre!(
                "Workspace tasks do not support specifying a service."
            ));
        }

        workspace
            .config_mut()
            .tasks
            .insert(task_name.clone(), command);
        workspace
            .save()
            .wrap_err("Failed to save workspace configuration")?;

        println!(
            "Task '{}' added to workspace '{}'.",
            task_name,
            workspace.config().name
        );
    } else {
        let mut project = get_project_for_cli(project_name, workspace_name)?;

        let task = if let Some(service) = service {
            Task::Compose { service, command }
        } else {
            Task::Raw(RawTask::Flat(command))
        };

        project
            .manifest_mut()
            .tasks
            .get_or_insert_with(Default::default)
            .insert(task_name.clone(), task);
        project
            .manifest()
            .save(project.manifest_path())
            .wrap_err("Failed to save project configuration")?;

        println!(
            "Task '{}' added to project '{}'.",
            task_name,
            project.manifest().project().name
        );
    }

    Ok(())
}

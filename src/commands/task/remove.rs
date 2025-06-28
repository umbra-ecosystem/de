use eyre::{Context, eyre};

use crate::{
    types::Slug,
    utils::{get_project_for_cli, get_workspace_for_cli},
};

pub fn remove(
    task_name: Slug,
    project_name: Option<Slug>,
    workspace_name: Option<Option<Slug>>,
) -> eyre::Result<()> {
    if workspace_name.is_some() {
        let mut workspace = get_workspace_for_cli(workspace_name)?;

        if workspace.config_mut().tasks.remove(&task_name).is_some() {
            workspace
                .save()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to save workspace configuration")?;
            println!(
                "Task '{}' removed from workspace '{}'.",
                task_name,
                workspace.config().name
            );
        } else {
            println!(
                "Task '{}' not found in workspace '{}'.",
                task_name,
                workspace.config().name
            );
        }
    } else {
        let mut project = get_project_for_cli(project_name, workspace_name)?;

        if project
            .manifest_mut()
            .tasks
            .get_or_insert_with(Default::default)
            .remove(&task_name)
            .is_some()
        {
            project
                .manifest()
                .save(project.manifest_path())
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to save project configuration")?;
            println!(
                "Task '{}' removed from project '{}'.",
                task_name,
                project.manifest().project().name
            );
        } else {
            println!(
                "Task '{}' not found in project '{}'.",
                task_name,
                project.manifest().project().name
            );
        }
    }

    Ok(())
}

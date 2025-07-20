use eyre::{Context, eyre};

use crate::{
    project::Project,
    types::Slug,
    utils::ui::UserInterface,
    workspace::{self, Workspace, WorkspaceProject},
};

/// Updates workspace registrations and project configurations.
pub fn update(all: bool, workspace: Option<Option<Slug>>) -> eyre::Result<()> {
    let ui = UserInterface::new();

    if all {
        update_all_workspaces(&ui)
    } else if let Some(workspace_name) = workspace {
        let workspace = if let Some(name) = workspace_name {
            Workspace::load_from_name(&name)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to load workspace")?
                .ok_or_else(|| eyre!("Workspace '{}' not found", name))?
        } else {
            Workspace::active()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to get active workspace")?
                .ok_or_else(|| eyre!("No active workspace found"))?
        };

        update_workspace(&ui, workspace)
    } else {
        update_current_project(&ui)
    }
}

/// Updates all workspaces by scanning for projects and validating existing registrations.
fn update_all_workspaces(ui: &UserInterface) -> eyre::Result<()> {
    ui.writeln("Updating all workspaces...")?;

    // Get all workspace config files
    let project_dirs = crate::utils::get_project_dirs()?;
    let workspaces_dir = project_dirs.config_local_dir().join("workspaces");

    if !workspaces_dir.exists() {
        ui.writeln("No workspaces directory found. Nothing to update.")?;
        return Ok(());
    }

    let mut updated_count = 0;
    let mut removed_count = 0;

    // Read all workspace files
    for entry in std::fs::read_dir(&workspaces_dir)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to read workspaces directory")?
    {
        let entry = entry
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to read workspace directory entry")?;

        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }

        let workspace_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<Slug>().ok());

        if let Some(workspace_name) = workspace_name {
            let workspace = Workspace::load_from_name(&workspace_name)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to load workspace '{workspace_name}'"))?
                .ok_or_else(|| eyre!("Workspace '{}' not found.", workspace_name))?;

            let (updated, removed) = update_workspace_internal(ui, workspace)?;

            updated_count += updated;
            removed_count += removed;
        } else {
            ui.writeln(&format!(
                "Skipping invalid workspace file: {}",
                path.display()
            ))?;
            continue;
        }
    }

    ui.writeln(&format!(
        "Updated {updated_count} projects across all workspaces"
    ))?;
    if removed_count > 0 {
        ui.writeln(&format!(
            "Removed {removed_count} stale project registrations"
        ))?;
    }

    Ok(())
}

/// Updates a specific workspace by name.
fn update_workspace(ui: &UserInterface, workspace: Workspace) -> eyre::Result<()> {
    let workspace_name = workspace.config().name.clone();

    ui.writeln(&format!("Updating workspace '{workspace_name}'..."))?;

    let (updated_count, removed_count) = update_workspace_internal(ui, workspace)?;

    ui.writeln(&format!(
        "Updated {updated_count} projects in workspace '{workspace_name}'"
    ))?;
    if removed_count > 0 {
        ui.writeln(&format!(
            "Removed {removed_count} stale project registrations"
        ))?;
    }

    Ok(())
}

/// Internal function that handles the actual workspace update logic.
fn update_workspace_internal(
    ui: &UserInterface,
    mut workspace: Workspace,
) -> eyre::Result<(usize, usize)> {
    let workspace_name = &workspace.config().name;

    let mut updated_count = 0;

    let mut add_projects = Vec::new();
    let mut remove_projects = Vec::new();

    // Validate existing project registrations
    let project_configs = workspace.config().projects.clone();
    for (project_name, project_config) in project_configs {
        let project_path = &project_config.dir;
        let manifest_path = project_path.join("de.toml");

        if !manifest_path.exists() {
            ui.warning_item(
                &format!(
                    "Removing stale project '{}' (manifest not found at {})",
                    project_name,
                    manifest_path.display()
                ),
                None,
            )?;
            remove_projects.push(project_name.clone());
            continue;
        }

        match Project::from_dir(project_path) {
            Ok(project) => {
                let current_manifest = project.manifest();

                // Check if the project still belongs to this workspace
                if current_manifest.project().workspace != *workspace_name {
                    ui.info_item(&format!(
                        "Removing project '{}' (moved to workspace '{}')",
                        project_name,
                        current_manifest.project().workspace
                    ))?;
                    remove_projects.push(project_name.clone());
                    continue;
                }

                // Check if project name has changed, and update if necessary
                if current_manifest.project().name != project_name {
                    ui.info_item(&format!(
                        "Updating project name '{}' -> '{}'",
                        project_name,
                        current_manifest.project().name
                    ))?;
                    remove_projects.push(project_name.clone());

                    // Add with new name to the new workspace
                    let project_entry = WorkspaceProject::new(project_path.clone())?;
                    add_projects.push((current_manifest.project().name.clone(), project_entry));
                    updated_count += 1;
                }
            }
            Err(e) => {
                eprintln!("  Failed to loading project '{project_name}' (failed to load: {e})");
            }
        }
    }

    // Apply changes: remove stale projects and add new/updated projects
    let removed_count = remove_projects.len();
    for project_name in remove_projects {
        workspace.remove_project(&project_name);
    }

    for (project_name, project_entry) in add_projects {
        workspace.add_project(project_name, project_entry);
    }

    workspace.save()?;

    Ok((updated_count, removed_count))
}

/// Updates the current project's workspace registration.
fn update_current_project(ui: &UserInterface) -> eyre::Result<()> {
    let project =
        Project::current()?.ok_or_else(|| eyre!("No de.toml found in current directory"))?;

    let project_name = &project.manifest().project().name;
    let workspace_name = &project.manifest().project().workspace;
    let project_path = project.dir();

    // Re-register the project to ensure workspace is up to date
    workspace::add_project_to_workspace(
        workspace_name.clone(),
        project_name.clone(),
        project_path.clone(),
    )
    .wrap_err("Failed to update project registration")?;

    // TODO: Remove from previous workspace if it exists

    ui.writeln(&format!("Updated project '{project_name}' registration"))?;

    Ok(())
}

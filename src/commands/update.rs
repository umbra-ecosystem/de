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

    ui.heading("Update Summary:")?;

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
    let project_dirs = crate::utils::get_project_dirs()?;
    let workspaces_dir = project_dirs.config_local_dir().join("workspaces");

    if !workspaces_dir.exists() {
        ui.error_item("No workspaces directory found. Nothing to update.", None)?;
        return Ok(());
    }

    let mut updated_count = 0;
    let mut removed_count = 0;
    let mut skipped_count = 0;

    // Collect per-workspace results for summary
    let mut workspace_summaries = Vec::new();

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

            ui.subheading(workspace_name.as_str())?;

            let (updated, removed, summary) =
                ui.indented(|ui| update_workspace_internal_verbose(ui, workspace))?;

            updated_count += updated;
            removed_count += removed;
            workspace_summaries.push(summary);
        } else {
            ui.error_item(
                &format!("Skipping invalid workspace file: {}", path.display()),
                None,
            )?;
            skipped_count += 1;
            continue;
        }
    }

    // Print summary for all workspaces

    if updated_count > 0 || removed_count > 0 || skipped_count > 0 {
        ui.new_line()?;
    }

    if updated_count > 0 {
        ui.info_item(&format!("Updated {updated_count} projects"))?;
    }
    if removed_count > 0 {
        ui.warning_item(&format!("Removed {removed_count} stale projects"), None)?;
    }
    if skipped_count > 0 {
        ui.error_item(
            &format!("Skipped {skipped_count} invalid workspace files"),
            None,
        )?;
    }

    Ok(())
}

/// Updates a specific workspace by name.
fn update_workspace(ui: &UserInterface, workspace: Workspace) -> eyre::Result<()> {
    let workspace_name = workspace.config().name.clone();

    ui.subheading(workspace_name.as_str())?;
    let (updated_count, removed_count, _summary) =
        update_workspace_internal_verbose(ui, workspace)?;

    if updated_count > 0 || removed_count > 0 {
        ui.new_line()?;
    }

    if updated_count > 0 {
        ui.info_item(&format!(
            "Updated {updated_count} projects in '{workspace_name}'"
        ))?;
    }
    if removed_count > 0 {
        ui.warning_item(&format!("Removed {removed_count} stale projects"), None)?;
    }
    if updated_count == 0 && removed_count == 0 {
        ui.success_item("No changes.", None)?;
    }

    Ok(())
}

/// Internal function that handles the actual workspace update logic, with verbose UI output.
fn update_workspace_internal_verbose(
    ui: &UserInterface,
    mut workspace: Workspace,
) -> eyre::Result<(usize, usize, String)> {
    let workspace_name = &workspace.config().name;

    let mut updated_count = 0;
    let mut removed_count = 0;

    let mut add_projects = Vec::new();
    let mut remove_projects = Vec::new();

    // Validate existing project registrations
    let project_configs = workspace.config().projects.clone();
    for (project_name, project_config) in project_configs {
        let project_path = &project_config.dir;
        let manifest_path = project_path.join("de.toml");

        if !manifest_path.exists() {
            ui.warning_item(
                &format!("Removed: {}", ui.theme.highlight(project_name.as_str())),
                None,
            )?;
            remove_projects.push(project_name.clone());
            removed_count += 1;
            continue;
        }

        match Project::from_dir(project_path) {
            Ok(project) => {
                let current_manifest = project.manifest();

                // Check if the project still belongs to this workspace
                if current_manifest.project().workspace != *workspace_name {
                    ui.info_item(&format!(
                        "Removed: {} (moved to '{}')",
                        ui.theme.highlight(project_name.as_str()),
                        ui.theme
                            .accent(current_manifest.project().workspace.as_str())
                    ))?;
                    remove_projects.push(project_name.clone());
                    removed_count += 1;
                    continue;
                }

                // Check if project name has changed, and update if necessary
                if current_manifest.project().name != project_name {
                    ui.info_item(&format!(
                        "Renamed: {} → {}",
                        ui.theme.highlight(project_name.as_str()),
                        ui.theme.accent(current_manifest.project().name.as_str()),
                    ))?;
                    remove_projects.push(project_name.clone());

                    // Add with new name to the new workspace
                    let project_entry = WorkspaceProject::new(project_path.clone())?;
                    add_projects.push((current_manifest.project().name.clone(), project_entry));
                    updated_count += 1;
                }
            }
            Err(e) => {
                ui.error_item(
                    &format!("Error: {} ({})", ui.theme.error(project_name.as_str()), e),
                    None,
                )?;
            }
        }
    }

    // Apply changes: remove stale projects and add new/updated projects
    for project_name in remove_projects {
        workspace.remove_project(&project_name);
    }

    for (project_name, project_entry) in add_projects {
        workspace.add_project(project_name, project_entry);
    }

    workspace.save()?;

    let summary = format!("Updated {updated_count}, removed {removed_count}");

    if updated_count == 0 && removed_count == 0 {
        ui.success_item("No changes.", None)?;
    } else {
        ui.info_item(&summary)?;
    }

    Ok((updated_count, removed_count, summary))
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

    ui.success_item(
        &format!(
            "Updated project: {}",
            ui.theme.highlight(project_name.as_str()),
        ),
        None,
    )?;

    Ok(())
}

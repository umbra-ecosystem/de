use std::path::PathBuf;

use crate::{
    project::Project,
    types::Slug,
    workspace::{DependencyGraph, Workspace, config::WorkspaceProject},
};
use eyre::{Context, eyre};

pub fn add_project_to_workspace(
    workspace_name: Slug,
    project_id: Slug,
    project_dir: PathBuf,
) -> eyre::Result<()> {
    let mut workspace = if let Some(workspace) =
        Workspace::load_from_name(&workspace_name).map_err(|e| eyre!(e))?
    {
        workspace
    } else {
        Workspace::new(workspace_name)?
    };

    let project = WorkspaceProject::new(project_dir)
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load workspace project")?;

    if let Some(existing_project) = workspace.config().projects.get(&project_id) {
        if existing_project.dir != project.dir {
            return Err(eyre!(
                "Project ID '{}' already exists with a different directory: {}",
                project_id,
                existing_project.dir.display()
            ));
        }
    }

    workspace.add_project(project_id, project);

    workspace
        .save()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to save workspace configuration")?;

    Ok(())
}

pub fn spin_up_workspace(workspace: &Workspace) -> eyre::Result<()> {
    let mut dependency_graph = DependencyGraph::new();
    let mut projects_map = std::collections::BTreeMap::new();

    // Load all projects and build dependency graph
    for (id, wp) in &workspace.config().projects {
        let project = Project::from_dir(&wp.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from directory {}", wp.dir.display())
            })?;

        let depends_on = project.manifest().project().depends_on.clone();
        dependency_graph.add_project(id.clone(), depends_on);
        projects_map.insert(id.clone(), project);
    }

    // Validate dependencies
    dependency_graph
        .validate_dependencies()
        .wrap_err("Failed to validate project dependencies")?;

    // Get startup order
    let startup_order = dependency_graph
        .resolve_startup_order()
        .wrap_err("Failed to resolve project startup order")?;

    let mut applied_projects = Vec::new();

    // Start projects in dependency order
    for project_id in startup_order {
        if let Some(project) = projects_map.get(&project_id) {
            println!("Spinning up project {project_id}:");

            let applied = project
                .docker_compose_up()
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to spin up project {} in workspace {}",
                        project_id,
                        workspace.config().name
                    )
                })?;

            if applied {
                applied_projects.push(project);
            }
        }
    }

    if applied_projects.is_empty() {
        println!("- (No projects to spin up)");
    }

    Ok(())
}

pub fn spin_down_workspace(workspace: &Workspace) -> eyre::Result<()> {
    let mut dependency_graph = DependencyGraph::new();
    let mut projects_map = std::collections::BTreeMap::new();

    // Load all projects and build dependency graph
    for (id, wp) in &workspace.config().projects {
        let project = Project::from_dir(&wp.dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to load project from directory {}", wp.dir.display())
            })?;

        let depends_on = project.manifest().project().depends_on.clone();
        dependency_graph.add_project(id.clone(), depends_on);
        projects_map.insert(id.clone(), project);
    }

    // Validate dependencies
    dependency_graph
        .validate_dependencies()
        .wrap_err("Failed to validate project dependencies")?;

    // Get shutdown order (reverse of startup order)
    let shutdown_order = dependency_graph
        .resolve_shutdown_order()
        .wrap_err("Failed to resolve project shutdown order")?;

    let mut applied_projects = Vec::new();

    // Stop projects in reverse dependency order
    for project_id in shutdown_order {
        if let Some(project) = projects_map.get(&project_id) {
            println!("Spinning down project {project_id}:");

            let applied = project
                .docker_compose_down()
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!(
                        "Failed to spin down project {} in workspace {}",
                        project_id,
                        workspace.config().name
                    )
                })?;

            if applied {
                applied_projects.push(project);
            }
        }
    }

    if applied_projects.is_empty() {
        println!("- (No projects to spin down)");
    }

    Ok(())
}

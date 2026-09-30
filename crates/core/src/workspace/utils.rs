use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    project::Project,
    types::Slug,
    workspace::{Workspace, config::WorkspaceProject},
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

    if let Some(existing_project) = workspace.config().projects.get(&project_id)
        && existing_project.dir != project.dir
    {
        return Err(eyre!(
            "Project ID '{}' already exists with a different directory: {}",
            project_id,
            existing_project.dir.display()
        ));
    }

    workspace.add_project(project_id, project);

    workspace
        .save()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to save workspace configuration")?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Dependencies before dependents.
    Startup,
    /// Dependents before dependencies.
    Shutdown,
}

/// Loads the workspace's projects, ordered by their `depends_on` graph.
pub fn ordered_projects(workspace: &Workspace, order: Order) -> eyre::Result<Vec<(Slug, Project)>> {
    let (dependency_graph, projects) = workspace
        .load_dependency_graph()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load dependency graph for workspace")?;

    let mut projects_map: BTreeMap<_, _> = projects
        .into_iter()
        .map(|p| (p.manifest().project.name.clone(), p))
        .collect();

    dependency_graph
        .validate_dependencies()
        .wrap_err("Failed to validate project dependencies")?;

    let names = match order {
        Order::Startup => dependency_graph.resolve_startup_order(),
        Order::Shutdown => dependency_graph.resolve_shutdown_order(),
    }
    .wrap_err("Failed to resolve project order")?;

    Ok(names
        .into_iter()
        .filter_map(|name| projects_map.remove(&name).map(|project| (name, project)))
        .collect())
}

pub fn spin_up_workspace(workspace: &Workspace) -> eyre::Result<()> {
    let mut applied = false;

    for (name, project) in ordered_projects(workspace, Order::Startup)? {
        println!("Spinning up project {name}:");
        applied |= project
            .docker_compose_up()
            .wrap_err_with(|| format!("Failed to spin up project {name}"))?;
    }

    if !applied {
        println!("- (No projects to spin up)");
    }

    Ok(())
}

pub fn spin_down_workspace(workspace: &Workspace) -> eyre::Result<()> {
    let mut applied = false;

    for (name, project) in ordered_projects(workspace, Order::Shutdown)? {
        println!("Spinning down project {name}:");
        applied |= project
            .docker_compose_down()
            .wrap_err_with(|| format!("Failed to spin down project {name}"))?;
    }

    if !applied {
        println!("- (No projects to spin down)");
    }

    Ok(())
}

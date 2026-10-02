use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    project::Project,
    types::Slug,
    workspace::{Workspace, config::WorkspaceProject, dependency::DependencyGraph},
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

/// The registered project ids in the order they start: dependencies first when the dependency
/// graph can be built, otherwise their registration order.
///
/// Unlike [`ordered_projects`] this never fails. A folder that has moved or a manifest that no
/// longer parses must not take the whole list down, because the screens that show a workspace's
/// state (the opening sequence, the health table) still have to show the other projects.
pub fn startup_order(workspace: &Workspace) -> Vec<Slug> {
    let mut graph = DependencyGraph::new();
    for (id, ws_project) in &workspace.config().projects {
        let depends_on = Project::from_dir(&ws_project.dir)
            .ok()
            .and_then(|p| p.manifest().project().depends_on.clone())
            .unwrap_or_default();
        graph.add_project(id.clone(), depends_on);
    }

    let registered: Vec<Slug> = workspace.config().projects.keys().cloned().collect();
    let Ok(mut order) = graph.resolve_startup_order() else {
        // A cycle (or anything else the sort refuses): registration order is at least stable.
        return registered;
    };
    order.retain(|id| workspace.config().projects.contains_key(id));
    for id in &registered {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    order
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::config::WorkspaceProject;
    use std::str::FromStr;

    fn slug(s: &str) -> Slug {
        Slug::from_str(s).unwrap()
    }

    /// A project folder with a `de.toml` (an empty manifest is a valid one).
    fn project(root: &std::path::Path, id: &str, body: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("de.toml"), body).unwrap();
        dir
    }

    fn workspace_with(projects: Vec<(&str, PathBuf)>) -> Workspace {
        let mut ws = Workspace::new(slug("shop")).unwrap();
        for (id, dir) in projects {
            ws.add_project(slug(id), WorkspaceProject { dir });
        }
        ws
    }

    #[test]
    fn startup_order_puts_dependencies_first() {
        let root = tempfile::tempdir().unwrap();
        let web = project(
            root.path(),
            "web",
            "[project]\nname = \"web\"\ndepends_on = [\"api\"]\n",
        );
        let api = project(
            root.path(),
            "api",
            "[project]\nname = \"api\"\ndepends_on = [\"db\"]\n",
        );
        let db = project(root.path(), "db", "[project]\nname = \"db\"\n");
        // Registered in the wrong order on purpose; the graph decides.
        let ws = workspace_with(
            vec![("web", web), ("api", api), ("db", db)],
        );

        let started = startup_order(&ws);
        let order: Vec<&str> = started.iter().map(Slug::as_str).collect();
        assert_eq!(order, ["db", "api", "web"]);
    }

    #[test]
    fn startup_order_falls_back_when_a_project_folder_is_gone() {
        let root = tempfile::tempdir().unwrap();
        let web = project(
            root.path(),
            "web",
            "[project]\nname = \"web\"\ndepends_on = [\"api\"]\n",
        );
        let missing = root.path().join("api");
        let ws = workspace_with(vec![("web", web), ("api", missing)]);

        let started = startup_order(&ws);
        let order: Vec<&str> = started.iter().map(Slug::as_str).collect();
        assert_eq!(order, ["api", "web"], "the registration order, nothing dropped");
    }

    #[test]
    fn startup_order_falls_back_on_a_dependency_that_is_not_in_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let web = project(
            root.path(),
            "web",
            "[project]\nname = \"web\"\ndepends_on = [\"ghost\"]\n",
        );
        let ws = workspace_with(vec![("web", web)]);

        let started = startup_order(&ws);
        let order: Vec<&str> = started.iter().map(Slug::as_str).collect();
        assert_eq!(order, ["web"]);
    }
}

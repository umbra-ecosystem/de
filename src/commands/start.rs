use dialoguer::{Select, theme::ColorfulTheme};
use eyre::{WrapErr, eyre};
use std::collections::BTreeSet;

use crate::{
    commands::{status::workspace_status, stop::stop_workspace},
    config::Config,
    project::Project,
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
    workspace::{Workspace, spin_up_workspace},
};

pub fn start(workspace_name: Option<Option<Slug>>) -> eyre::Result<()> {
    let ui = UserInterface::new();

    check_for_active_workspace(&ui)?;

    if let Some(workspace_name) = workspace_name {
        // Start entire workspace
        let workspace = get_workspace_for_cli(Some(workspace_name))
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get workspace for CLI")?;

        spin_up_workspace(&workspace)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to spin up workspace")?;

        Config::mutate_persisted(|config| {
            config.set_active_workspace(Some(workspace.config().name.clone()));
        })?;

        // We ignore the error here because we want to proceed even if the status check fails
        ui.new_line()?;
        let _ = workspace_status(&ui, &workspace);
    } else {
        // Start current project and its dependencies
        let project = Project::current()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found"))?;

        let workspace_name = project.manifest().project().workspace.clone();

        let workspace = Workspace::load_from_name(&workspace_name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to load workspace")?
            .ok_or_else(|| eyre!("Workspace {} not found", workspace_name))?;

        spin_up_project_and_dependencies(&ui, &workspace, &project.manifest().project().name)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to spin up project and dependencies")?;

        Config::mutate_persisted(|config| {
            config.set_active_workspace(Some(workspace_name));
        })?;

        // We ignore the error here because we want to proceed even if the status check fails
        ui.new_line()?;
        let _ = workspace_status(&ui, &workspace);
    }

    Ok(())
}

fn check_for_active_workspace(ui: &UserInterface) -> eyre::Result<()> {
    let working_workspace = Workspace::working()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get working workspace")?;

    let Some(working_workspace) = working_workspace else {
        return Ok(());
    };

    ui.heading("Old Workspace")?;

    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "A workspace ({}) is already active. How do you wish to proceed?",
            ui.theme.accent(working_workspace.config().name.as_str())
        ))
        .items(&[
            "Abort starting a new workspace",
            "Deactivate the current workspace and start the new one",
            "Start the new workspace alongside the current one",
        ])
        .default(0)
        .interact()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to prompt for workspace conflict resolution")?;

    match choice {
        0 => {
            return Err(eyre!("Start operation aborted by user"));
        }
        1 => {
            ui.new_line()?;

            let stopped = stop_workspace(ui, working_workspace)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to stop current workspace")?;

            if !stopped {
                return Err(eyre!("Stop current workspace aborted"));
            }
        }
        2 => {}
        _ => unreachable!(),
    }

    ui.new_line()?;

    Ok(())
}

fn spin_up_project_and_dependencies(
    ui: &UserInterface,
    workspace: &Workspace,
    project_name: &Slug,
) -> eyre::Result<()> {
    let (dependency_graph, projects) = workspace
        .load_dependency_graph()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to load dependency graph for workspace")?;

    let projects_map: std::collections::BTreeMap<_, _> = projects
        .into_iter()
        .map(|p| (p.manifest().project.name.clone(), p))
        .collect();

    // Validate dependencies
    dependency_graph
        .validate_dependencies()
        .wrap_err("Failed to validate project dependencies")?;

    // Get all projects that need to be started (current project and its dependencies)
    let mut projects_to_start = BTreeSet::new();
    collect_dependencies(&dependency_graph, project_name, &mut projects_to_start);

    // Get startup order for all projects
    let startup_order = dependency_graph
        .resolve_startup_order()
        .wrap_err("Failed to resolve project startup order")?;

    let mut applied_projects = Vec::new();

    // Start only the projects we need, in dependency order
    for project_id in startup_order {
        if projects_to_start.contains(&project_id)
            && let Some(project) = projects_map.get(&project_id)
        {
            ui.writeln(&ui.theme.bold(&format!("Spinning up project {project_id}:")))?;

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
        ui.warning_item("No projects to spin up", None)?;
    }

    Ok(())
}

fn collect_dependencies(
    dependency_graph: &crate::workspace::DependencyGraph,
    project_name: &Slug,
    collected: &mut BTreeSet<Slug>,
) {
    // Add the current project
    collected.insert(project_name.clone());

    // Add its dependencies recursively
    if let Some(dependencies) = dependency_graph.get_dependencies(project_name) {
        for dep in dependencies {
            if !collected.contains(dep) {
                collect_dependencies(dependency_graph, dep, collected);
            }
        }
    }
}

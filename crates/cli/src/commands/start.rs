use dialoguer::{Select, theme::ColorfulTheme};
use eyre::{WrapErr, eyre};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    commands::{stop::stop_workspace, workspace::report},
    config::Config,
    project::Project,
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
    workspace::{Workspace, health, registry, spin_up_workspace},
};

pub fn start(workspace_name: Option<Option<Slug>>, yes: bool) -> eyre::Result<()> {
    let ui = UserInterface::new();

    check_for_active_workspace(&ui, yes)?;

    if let Some(workspace_name) = workspace_name {
        // Start entire workspace
        let workspace = get_workspace_for_cli(Some(workspace_name))
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get workspace for CLI")?;

        // Opening a workspace records when it was last used: the lists are ordered by it.
        // Done before anything starts, so a workspace file that cannot be written fails the
        // command while nothing has run yet.
        registry::stamp_last_used(&workspace.config().name, now())?;

        spin_up_workspace(&workspace)
            .map_err(|e| eyre!(e))
            .wrap_err("The workspace could not be started")?;

        Config::mutate_persisted(|config| {
            config.set_active_workspace(Some(workspace.config().name.clone()));
        })?;

        ui.new_line()?;
        show_containers(&ui, &workspace)?;
    } else {
        // Start current project and its dependencies
        let project = Project::current_or_inferred()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to get current project")?
            .ok_or_else(|| eyre!("No current project found"))?;

        // Show hint if inferred
        if project.is_inferred() {
            ui.info_item("💡 Tip: Run 'de init' to create a de.toml for more features (workspace management, tasks, dependencies)")?;
            ui.new_line()?;
        }

        // For inferred projects, skip workspace/dependency logic
        if project.is_inferred() {
            ui.writeln(
                &ui.theme
                    .bold(&format!("Starting {}:", project.manifest().project().name)),
            )?;

            let started = project
                .docker_compose_up()
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to start docker compose services")?;

            if !started {
                ui.warning_item("No docker-compose file found", None)?;
            }
        } else {
            // Existing logic for configured projects with workspace/dependencies
            let workspace_name = project.manifest().project().workspace.clone();
            let workspace = crate::workspace::registry::load_workspace(&workspace_name)
                .map_err(|e| eyre!(e))?;

            spin_up_project_and_dependencies(&ui, &workspace, &project.manifest().project().name)
                .map_err(|e| eyre!(e))
                .wrap_err("Failed to spin up project and dependencies")?;

            Config::mutate_persisted(|config| {
                config.set_active_workspace(Some(workspace_name));
            })?;

            ui.new_line()?;
            show_containers(&ui, &workspace)?;
        }
    }

    Ok(())
}

/// Prints the services of every project in the workspace, as they are right now.
fn show_containers(ui: &UserInterface, workspace: &Workspace) -> eyre::Result<()> {
    let health = health::check_workspace(workspace, now());
    ui.new_line()?;
    report(ui, &health)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn check_for_active_workspace(ui: &UserInterface, yes: bool) -> eyre::Result<()> {
    let working_workspace = Workspace::working()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get working workspace")?;

    let Some(working_workspace) = working_workspace else {
        return Ok(());
    };

    ui.heading("Old Workspace")?;

    let choice = if yes {
        // When --yes is used, default to option 1 (deactivate current and start new)
        1
    } else {
        Select::with_theme(&ColorfulTheme::default())
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
            .wrap_err("Failed to prompt for workspace conflict resolution")?
    };

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

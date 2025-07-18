use console::style;
use eyre::eyre;
use itertools::Itertools;
use std::process::Command;

use crate::{
    project::Project,
    types::Slug,
    utils::{formatter::Formatter, theme::Theme},
    workspace::{DependencyGraphError, Workspace},
};

#[derive(Debug)]
struct DiagnosticResult {
    errors: u32,
    warnings: u32,
}

impl DiagnosticResult {
    fn new() -> Self {
        Self {
            errors: 0,
            warnings: 0,
        }
    }

    fn add_success(&mut self, formatter: &Formatter, message: String) -> eyre::Result<()> {
        formatter.success(&message)?;
        Ok(())
    }

    fn add_error(
        &mut self,
        formatter: &Formatter,
        message: String,
        suggestion: Option<String>,
    ) -> eyre::Result<()> {
        self.errors += 1;
        formatter.error(&message, suggestion.as_deref())?;
        Ok(())
    }

    fn add_error_group(
        &mut self,
        formatter: &Formatter,
        heading: String,
        messages: Vec<String>,
        suggestion: Option<String>,
    ) -> eyre::Result<()> {
        if messages.is_empty() {
            return Ok(());
        }

        self.errors += 1;
        formatter.error_group(&heading, &messages, suggestion.as_deref())?;
        Ok(())
    }

    fn add_warning(
        &mut self,
        formatter: &Formatter,
        message: String,
        suggestion: Option<String>,
    ) -> eyre::Result<()> {
        self.warnings += 1;
        formatter.warning(&message, suggestion.as_deref())?;
        Ok(())
    }

    fn add_info(&mut self, formatter: &Formatter, message: String) -> eyre::Result<()> {
        formatter.info(&message)?;
        Ok(())
    }
}

pub fn doctor(workspace_name: Option<Slug>) -> eyre::Result<()> {
    let formatter = Formatter::new();
    let theme = crate::utils::theme::Theme::new();

    // Check system dependencies
    formatter.heading("System Dependencies:")?;
    let system_result = check_system_dependencies(&formatter)?;
    println!();

    // Check project configuration
    // We don't want to show the project in doctor if its not in the current workspace
    let project = Project::current();
    let project_result = if workspace_name.as_ref()
        .map(|workspace_name| matches!(project, Ok(Some(project)) if &project.manifest().project().workspace == workspace_name))
        .unwrap_or(true)
    {
        formatter.heading("Project Configuration:")?;
        let project_result = check_project_configuration(&formatter, &theme)?;
        println!();
        Some(project_result)
    } else {
        None
    };

    // Check workspace configuration
    formatter.heading("Workspace Configuration:")?;
    let workspace_result = check_workspace_configuration(&formatter, workspace_name.as_ref())?;

    // Calculate totals and print status
    let total_errors = system_result.errors
        + project_result
            .as_ref()
            .map(|v| v.errors)
            .unwrap_or_default()
        + workspace_result.errors;
    let total_warnings = system_result.warnings
        + project_result
            .as_ref()
            .map(|v| v.warnings)
            .unwrap_or_default()
        + workspace_result.warnings;

    println!();
    formatter.heading("Status:")?;
    if total_errors == 0 && total_warnings == 0 {
        formatter.success(
            &style("All systems operational")
                .fg(theme.success_color)
                .to_string(),
        )?;
    } else {
        if total_errors > 0 {
            formatter.error(
                &format!(
                    "{} error(s) found",
                    style(total_errors).fg(theme.error_color).bold()
                ),
                None,
            )?;
        }
        if total_warnings > 0 {
            formatter.warning(
                &format!(
                    "{} warning(s) found",
                    style(total_warnings).fg(theme.warning_color).bold()
                ),
                None,
            )?;
        }

        println!();
        println!(
            "Run {} again after addressing issues to verify fixes.",
            style("de doctor").fg(theme.accent_color)
        );
    }

    Ok(())
}

fn check_system_dependencies(formatter: &Formatter) -> eyre::Result<DiagnosticResult> {
    let mut result = DiagnosticResult::new();

    // Check Docker
    match check_docker() {
        Ok(version) => result.add_success(formatter, format!("Docker: {}", version.trim()))?,
        Err(e) => result.add_error(
            formatter,
            format!("Docker: {e}"),
            Some("Install from https://docs.docker.com/get-docker/".to_string()),
        )?,
    }

    // Check Docker Compose
    match check_docker_compose() {
        Ok(version) => {
            result.add_success(formatter, format!("Docker Compose: {}", version.trim()))?
        }
        Err(e) => result.add_error(
            formatter,
            format!("Docker Compose: {e}"),
            Some("Install from https://docs.docker.com/compose/install/".to_string()),
        )?,
    }

    Ok(result)
}

fn check_project_configuration(
    formatter: &Formatter,
    theme: &Theme,
) -> eyre::Result<DiagnosticResult> {
    let mut result = DiagnosticResult::new();

    match Project::current() {
        Ok(Some(project)) => {
            result.add_success(
                formatter,
                format!("Project: {}", project.manifest().project().name),
            )?;
            check_project_details(formatter, theme, &project, &mut result)?;
        }
        Ok(None) => {
            result.add_warning(
                formatter,
                "Not in a de project directory".to_string(),
                Some("Run 'de init' to initialize a project here".to_string()),
            )?;
        }
        Err(e) => {
            result.add_error(formatter, format!("Project check failed: {e}"), None)?;
        }
    }

    Ok(result)
}

fn check_workspace_configuration(
    formatter: &Formatter,
    workspace_name: Option<&Slug>,
) -> eyre::Result<DiagnosticResult> {
    let mut result = DiagnosticResult::new();

    let workspace = if let Some(name) = workspace_name {
        Workspace::load_from_name(name)
    } else {
        Workspace::active()
    };

    match workspace {
        Ok(Some(workspace)) => {
            result.add_success(formatter, format!("Workspace: {}", workspace.config().name))?;
            check_workspace_details(formatter, &workspace, &mut result)?;
        }
        Ok(None) => {
            if workspace_name.is_some() {
                result.add_error(
                    formatter,
                    "Workspace not found".to_string(),
                    Some("Check if the workspace name is correct or run 'de init' to create a new workspace".to_string())
                )?;
            } else {
                result.add_warning(
                    formatter,
                    "No active workspace found".to_string(),
                    Some("Initialize a project or set an active workspace".to_string()),
                )?;
            }
        }
        Err(e) => {
            result.add_error(formatter, format!("Workspace check failed: {e}"), None)?;
        }
    }

    Ok(result)
}

fn check_docker() -> eyre::Result<String> {
    let output = Command::new("docker")
        .arg("--version")
        .output()
        .map_err(|e| eyre!("Failed to execute docker command: {}", e))?;

    if !output.status.success() {
        return Err(eyre!("Docker command failed"));
    }

    let version = String::from_utf8(output.stdout)
        .map_err(|e| eyre!("Failed to parse docker version output: {}", e))?
        .trim()
        .to_string();

    // Test if Docker daemon is running
    let ping_output = Command::new("docker")
        .arg("info")
        .output()
        .map_err(|e| eyre!("Failed to ping Docker daemon: {}", e))?;

    if !ping_output.status.success() {
        return Err(eyre!("Docker daemon is not running"));
    }

    Ok(version)
}

fn check_docker_compose() -> eyre::Result<String> {
    // Try docker-compose first (standalone)
    let output = Command::new("docker-compose").arg("--version").output();

    if let Ok(output) = output {
        if output.status.success() {
            let version = String::from_utf8(output.stdout)
                .map_err(|e| eyre!("Failed to parse docker-compose version output: {}", e))?
                .trim()
                .to_string();
            return Ok(version);
        }
    }

    // Try docker compose (plugin)
    let output = Command::new("docker")
        .arg("compose")
        .arg("version")
        .output()
        .map_err(|e| eyre!("Failed to execute docker compose command: {}", e))?;

    if !output.status.success() {
        return Err(eyre!("Docker Compose is not available"));
    }

    let version = String::from_utf8(output.stdout)
        .map_err(|e| eyre!("Failed to parse docker compose version output: {}", e))?
        .trim()
        .to_string();

    Ok(version)
}

fn check_project_details(
    formatter: &Formatter,
    theme: &Theme,
    project: &Project,
    result: &mut DiagnosticResult,
) -> eyre::Result<()> {
    use crate::project::Task;
    use std::process::Command;

    // Check if project directory exists
    if !project.dir().exists() {
        result.add_error(
            formatter,
            format!("Project directory missing: {}", project.dir().display()),
            None,
        )?;
    }

    // Check if de.toml exists and is readable
    if !project.manifest_path().exists() {
        result.add_error(
            formatter,
            "Project manifest (de.toml) missing".to_string(),
            None,
        )?;
    }

    // Track Compose services for later check
    let mut compose_services: Option<Vec<String>> = None;

    // Check Docker Compose file if configured
    match project.docker_compose_path() {
        Ok(Some(compose_path)) => {
            if let Err(e) = validate_docker_compose(&compose_path) {
                result.add_error(formatter, format!("Docker Compose file invalid: {e}"), None)?;
            } else {
                result.add_success(
                    formatter,
                    format!(
                        "Docker Compose file: {}",
                        compose_path.file_name().unwrap().to_string_lossy()
                    ),
                )?;

                // Try to get list of services from docker-compose config --services
                let output = Command::new("docker-compose")
                    .arg("-f")
                    .arg(&compose_path)
                    .arg("config")
                    .arg("--services")
                    .output();

                let services = if let Ok(output) = &output {
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let v: Vec<String> = stdout.lines().map(|s| s.trim().to_string()).collect();
                        if !v.is_empty() { Some(v) } else { None }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // Fallback to docker compose (plugin) if standalone fails
                let services = if services.is_none() {
                    let output = Command::new("docker")
                        .arg("compose")
                        .arg("-f")
                        .arg(&compose_path)
                        .arg("config")
                        .arg("--services")
                        .output();
                    if let Ok(output) = &output {
                        if output.status.success() {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            let v: Vec<String> =
                                stdout.lines().map(|s| s.trim().to_string()).collect();
                            if !v.is_empty() { Some(v) } else { None }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    services
                };

                compose_services = services;
            }
        }
        Ok(None) => {
            result.add_info(formatter, theme.dim("Docker Compose: not configured"))?;
        }
        Err(e) => {
            result.add_error(formatter, format!("Docker Compose check failed: {e}"), None)?;
        }
    }

    // Check project dependencies
    if let Some(depends_on) = &project.manifest().project().depends_on {
        result.add_info(formatter, format!("Dependencies: {}", depends_on.len()))?;

        // If we're in a workspace context, validate dependencies
        if let Ok(Some(workspace)) =
            Workspace::load_from_name(&project.manifest().project.workspace)
        {
            let missing_dependencies: Vec<_> = depends_on
                .iter()
                .filter(|dep| !workspace.config().projects.contains_key(*dep))
                .map(|dep| dep.to_string())
                .collect();

            if !missing_dependencies.is_empty() {
                result.add_error(
                    formatter,
                    format!("Missing dependencies: {}", missing_dependencies.join(", ")),
                    Some("Ensure all required projects are added to the workspace".to_string()),
                )?;
            } else {
                let mut depends_on = depends_on.iter().map(|d| d.to_string());
                result.add_success(
                    formatter,
                    format!("All dependencies found: {}", depends_on.join(", ")),
                )?;
            }
        } else {
            result.add_error(
                formatter,
                "Cannot find workspace of this project".to_string(),
                Some(
                    "Ensure this workspace is initiated either with `de init` or `de scan`."
                        .to_string(),
                ),
            )?;
        }
    } else {
        result.add_info(formatter, theme.dim("Dependencies: none"))?;
    }

    // Check if project has any tasks defined
    let task_count = project
        .manifest()
        .tasks
        .as_ref()
        .map(|t| t.len())
        .unwrap_or(0);
    if task_count == 0 {
        result.add_warning(
            formatter,
            "No tasks defined".to_string(),
            Some("Add tasks to your de.toml".to_string()),
        )?;
    } else {
        result.add_success(formatter, format!("Tasks: {task_count} defined"))?;
    }

    // Check if Compose tasks reference missing services or if no Compose file exists
    if let Some(tasks) = project.manifest().tasks.as_ref() {
        if let Some(services) = compose_services.as_ref() {
            let service_set: std::collections::HashSet<_> = services.iter().collect();
            for (task_name, task) in tasks {
                if let Task::Compose { service, .. } = task {
                    if !service_set.contains(&service) {
                        result.add_error(
                            formatter,
                            format!(
                                "Task '{task_name}' references missing Docker Compose service '{service}'"
                            ),
                            Some(
                                "Check your de.toml and docker-compose.yml for consistency"
                                    .to_string(),
                            ),
                        )?;
                    }
                }
            }
        } else {
            // No Compose file found, but there are Compose tasks
            for (task_name, task) in tasks {
                if let Task::Compose { service, .. } = task {
                    result.add_error(
                        formatter,
                        format!(
                            "Task '{task_name}' references Docker Compose service '{service}' but no Docker Compose file is configured or found"
                        ),
                        Some(
                            "Add a docker-compose.yml or configure the docker_compose path in de.toml".to_string(),
                        ),
                    )?;
                }
            }
        }
    }

    // Check .env file
    let env_file = project.dir().join(".env");
    if env_file.exists() {
        result.add_success(formatter, "Environment file: .env".to_string())?;
    } else {
        result.add_info(formatter, theme.dim("Environment file: not found"))?;
    }

    Ok(())
}

fn check_workspace_details(
    formatter: &Formatter,
    workspace: &Workspace,
    result: &mut DiagnosticResult,
) -> eyre::Result<()> {
    let config = workspace.config();
    let project_count = config.projects.len();

    if project_count == 0 {
        result.add_warning(
            formatter,
            "Workspace has no projects".to_string(),
            Some("Run 'de scan' to discover projects or 'de init' to create new ones".to_string()),
        )?;
    } else {
        result.add_success(formatter, format!("Projects: {project_count} registered"))?;

        // Check if projects still exist
        let mut valid_projects = 0;
        let mut invalid_projects = 0;

        for (project_id, workspace_project) in &config.projects {
            if workspace_project.dir.exists() {
                valid_projects += 1;
            } else {
                invalid_projects += 1;
                result.add_error(
                    formatter,
                    format!(
                        "Missing: {} ({})",
                        project_id,
                        workspace_project.dir.display()
                    ),
                    None,
                )?;
            }
        }

        if invalid_projects > 0 {
            result.add_warning(
                formatter,
                format!("{invalid_projects} project(s) have missing directories"),
                Some("Run 'de update' to clean up workspace configuration".to_string()),
            )?;
        }

        if valid_projects > 0 && invalid_projects == 0 {
            result.add_success(formatter, "All project directories found".to_string())?;
        }

        // Check for task name conflicts
        check_for_conflicts(formatter, workspace, result)?;

        // Check for dependency issues
        check_for_dependency_issues(formatter, workspace, result)?;
    }
    Ok(())
}

fn check_for_conflicts(
    formatter: &Formatter,
    workspace: &Workspace,
    result: &mut DiagnosticResult,
) -> eyre::Result<()> {
    let config = workspace.config();
    let project_names: std::collections::HashSet<_> = config.projects.keys().collect();
    let workspace_task_names: std::collections::HashSet<_> = config.tasks.keys().collect();

    // Collect all project task names
    let mut all_project_task_names: std::collections::HashSet<Slug> =
        std::collections::HashSet::new();
    for (project_id, workspace_project) in &config.projects {
        if !workspace_project.dir.exists() {
            continue;
        }

        let project = match Project::from_dir(&workspace_project.dir) {
            Ok(project) => project,
            Err(e) => {
                result.add_error(
                    formatter,
                    format!("Failed to load project {project_id}: {e}"),
                    None,
                )?;
                continue;
            }
        };

        if let Some(tasks) = project.tasks() {
            for task_name in tasks.keys() {
                all_project_task_names.insert(task_name.clone());
            }
        }
    }

    // Check for conflicts between project task names and project names
    for task_name in &all_project_task_names {
        if project_names.contains(task_name) {
            result.add_warning(
                formatter,
                format!("Project task '{task_name}' conflicts with a project name."),
                Some("Consider renaming the task or project to avoid ambiguity.".to_string()),
            )?;
        }
    }

    // Check for conflicts between workspace task names and project names
    for task_name in &workspace_task_names {
        if project_names.contains(task_name) {
            result.add_warning(
                formatter,
                format!("Workspace task '{task_name}' conflicts with a project name."),
                Some("Consider renaming the task or project to avoid ambiguity.".to_string()),
            )?;
        }
    }

    // Highlight workspace tasks that override project tasks
    for task_name in &workspace_task_names {
        if all_project_task_names.contains(task_name) {
            result.add_info(
                formatter,
                format!(
                    "Workspace task '{task_name}' overrides a project task with the same name."
                ),
            )?;
        }
    }
    Ok(())
}

fn check_for_dependency_issues(
    formatter: &Formatter,
    workspace: &Workspace,
    result: &mut DiagnosticResult,
) -> eyre::Result<()> {
    let (dependency_graph, _) = match workspace.load_dependency_graph() {
        Ok(graph) => graph,
        Err(e) => {
            result.add_error(
                formatter,
                format!("Failed to load dependency graph: {e}"),
                Some("Ensure all projects are properly configured in the workspace".to_string()),
            )?;

            return Ok(());
        }
    };

    // Check for circular dependencies first (more critical)
    match dependency_graph.resolve_startup_order() {
        Ok(_) => {
            result.add_success(formatter, "Dependency order is valid".to_string())?;
        }
        Err(DependencyGraphError::CircularDependency(projects)) => {
            let projects_str = projects.iter().map(|p| p.as_str()).join(", ");
            result.add_error(
                formatter,
                format!("Circular dependency detected: {projects_str}"),
                Some("Refactor your dependencies to remove circular references".to_string()),
            )?;
        }
        Err(DependencyGraphError::MissingDependencies(_)) => {
            // This will be handled in the next check
        }
    }

    // Validate dependencies are available
    match dependency_graph.validate_dependencies() {
        Ok(()) => {
            result.add_success(formatter, "All dependencies are available".to_string())?;
        }
        Err(DependencyGraphError::MissingDependencies(dependencies)) => {
            let grouped = dependencies
                .into_iter()
                .chunk_by(|(key, _)| key.clone())
                .into_iter()
                .map(|(key, items)| {
                    let deps: Vec<_> = items.into_iter().map(|(_, dep)| dep).collect();
                    (key, deps)
                })
                .map(|(key, deps)| {
                    format!("{}: {}", key, deps.iter().map(|d| d.as_str()).join(", "))
                })
                .collect::<Vec<_>>();

            result.add_error_group(
                formatter,
                "Missing Dependencies".to_string(),
                grouped,
                Some("Ensure all required projects are added to the workspace".to_string()),
            )?;
        }
        Err(DependencyGraphError::CircularDependency(_)) => {
            // This should not happen here, already handled above
            result.add_error(
                formatter,
                "Unexpected circular dependency detected".to_string(),
                Some("This should not happen, please report this issue".to_string()),
            )?;
        }
    }

    Ok(())
}

fn validate_docker_compose(compose_path: &std::path::Path) -> eyre::Result<()> {
    let output = Command::new("docker-compose")
        .arg("-f")
        .arg(compose_path)
        .arg("config")
        .arg("--quiet")
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            return Ok(());
        }
    }

    // Try with docker compose plugin
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("config")
        .arg("--quiet")
        .output()
        .map_err(|e| eyre!("Failed to validate compose file: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!("Compose file validation failed: {}", stderr));
    }

    Ok(())
}

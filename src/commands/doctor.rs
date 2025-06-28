use eyre::eyre;
use std::process::Command;

use crate::{project::Project, types::Slug, workspace::Workspace};

#[derive(Debug)]
struct DiagnosticResult {
    errors: u32,
    warnings: u32,
    details: Vec<String>,
}

impl DiagnosticResult {
    fn new() -> Self {
        Self {
            errors: 0,
            warnings: 0,
            details: Vec::new(),
        }
    }

    fn add_success(&mut self, message: String) {
        self.details.push(format!("  ✓ {}", message));
    }

    fn add_error(&mut self, message: String, suggestion: Option<String>) {
        self.errors += 1;
        self.details.push(format!("  ✗ {}", message));
        if let Some(suggestion) = suggestion {
            self.details.push(format!("    → {}", suggestion));
        }
    }

    fn add_warning(&mut self, message: String, suggestion: Option<String>) {
        self.warnings += 1;
        self.details.push(format!("  ! {}", message));
        if let Some(suggestion) = suggestion {
            self.details.push(format!("    → {}", suggestion));
        }
    }

    fn add_info(&mut self, message: String) {
        self.details.push(format!("  - {}", message));
    }
}

pub fn doctor(workspace_name: Option<Slug>) -> eyre::Result<()> {
    // Check system dependencies
    println!("System Dependencies:");
    let system_result = check_system_dependencies();
    for detail in &system_result.details {
        println!("{}", detail);
    }
    println!();

    // Check project configuration
    // We don't want to show the project in doctor if its not in the current workspace
    let project = Project::current();
    let project_result = if workspace_name.as_ref()
        .map(|workspace_name| matches!(project, Ok(Some(project)) if &project.manifest().project().workspace == workspace_name))
        .unwrap_or(true)
    {
        println!("Project Configuration:");
        let project_result = check_project_configuration();
        for detail in &project_result.details {
            println!("{}", detail);
        }
        println!();
        Some(project_result)
    } else {
        None
    };

    // Check workspace configuration
    println!("Workspace Configuration:");
    let workspace_result = check_workspace_configuration(workspace_name.as_ref());
    for detail in &workspace_result.details {
        println!("{}", detail);
    }

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
    println!("Status:");
    if total_errors == 0 && total_warnings == 0 {
        println!("  All systems operational");
    } else {
        if total_errors > 0 {
            println!("  {} error(s) found", total_errors);
        }
        if total_warnings > 0 {
            println!("  {} warning(s) found", total_warnings);
        }
        println!();
        println!("Run 'de doctor' again after addressing issues to verify fixes.");
    }

    Ok(())
}

fn check_system_dependencies() -> DiagnosticResult {
    let mut result = DiagnosticResult::new();

    // Check Docker
    match check_docker() {
        Ok(version) => result.add_success(format!("Docker: {}", version.trim())),
        Err(e) => result.add_error(
            format!("Docker: {}", e),
            Some("Install from https://docs.docker.com/get-docker/".to_string()),
        ),
    }

    // Check Docker Compose
    match check_docker_compose() {
        Ok(version) => result.add_success(format!("Docker Compose: {}", version.trim())),
        Err(e) => result.add_error(
            format!("Docker Compose: {}", e),
            Some("Install from https://docs.docker.com/compose/install/".to_string()),
        ),
    }

    result
}

fn check_project_configuration() -> DiagnosticResult {
    let mut result = DiagnosticResult::new();

    match Project::current() {
        Ok(Some(project)) => {
            result.add_success(format!("Project: {}", project.manifest().project().name));
            check_project_details(&project, &mut result);
        }
        Ok(None) => {
            result.add_warning(
                "Not in a de project directory".to_string(),
                Some("Run 'de init' to initialize a project here".to_string()),
            );
        }
        Err(e) => {
            result.add_error(format!("Project check failed: {}", e), None);
        }
    }

    result
}

fn check_workspace_configuration(workspace_name: Option<&Slug>) -> DiagnosticResult {
    let mut result = DiagnosticResult::new();

    let workspace = if let Some(name) = workspace_name {
        Workspace::load_from_name(name)
    } else {
        Workspace::active()
    };

    match workspace {
        Ok(Some(workspace)) => {
            result.add_success(format!("Workspace: {}", workspace.config().name));
            check_workspace_details(&workspace, &mut result);
        }
        Ok(None) => {
            if workspace_name.is_some() {
                result.add_error(
                    "Workspace not found".to_string(),
                    Some("Check if the workspace name is correct or run 'de init' to create a new workspace".to_string()
                ));
            } else {
                result.add_warning(
                    "No active workspace found".to_string(),
                    Some("Initialize a project or set an active workspace".to_string()),
                );
            }
        }
        Err(e) => {
            result.add_error(format!("Workspace check failed: {}", e), None);
        }
    }

    result
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

fn check_project_details(project: &Project, result: &mut DiagnosticResult) {
    use crate::project::Task;
    use std::process::Command;

    // Check if project directory exists
    if !project.dir().exists() {
        result.add_error(
            format!("Project directory missing: {}", project.dir().display()),
            None,
        );
    }

    // Check if de.toml exists and is readable
    if !project.manifest_path().exists() {
        result.add_error("Project manifest (de.toml) missing".to_string(), None);
    }

    // Track Compose services for later check
    let mut compose_services: Option<Vec<String>> = None;

    // Check Docker Compose file if configured
    match project.docker_compose_path() {
        Ok(Some(compose_path)) => {
            if let Err(e) = validate_docker_compose(&compose_path) {
                result.add_error(format!("Docker Compose file invalid: {}", e), None);
            } else {
                result.add_success(format!(
                    "Docker Compose file: {}",
                    compose_path.file_name().unwrap().to_string_lossy()
                ));

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
            result.add_info("Docker Compose: not configured".to_string());
        }
        Err(e) => {
            result.add_error(format!("Docker Compose check failed: {}", e), None);
        }
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
            "No tasks defined".to_string(),
            Some("Add tasks to your de.toml".to_string()),
        );
    } else {
        result.add_success(format!("Tasks: {} defined", task_count));
    }

    // Check if Compose tasks reference missing services or if no Compose file exists
    if let Some(tasks) = project.manifest().tasks.as_ref() {
        if let Some(services) = compose_services.as_ref() {
            let service_set: std::collections::HashSet<_> = services.iter().collect();
            for (task_name, task) in tasks {
                if let Task::Compose { service, .. } = task {
                    if !service_set.contains(&service) {
                        result.add_error(
                            format!(
                                "Task '{}' references missing Docker Compose service '{}'",
                                task_name, service
                            ),
                            Some(
                                "Check your de.toml and docker-compose.yml for consistency"
                                    .to_string(),
                            ),
                        );
                    }
                }
            }
        } else {
            // No Compose file found, but there are Compose tasks
            for (task_name, task) in tasks {
                if let Task::Compose { service, .. } = task {
                    result.add_error(
                        format!(
                            "Task '{}' references Docker Compose service '{}' but no Docker Compose file is configured or found",
                            task_name, service
                        ),
                        Some(
                            "Add a docker-compose.yml or configure the docker_compose path in de.toml".to_string(),
                        ),
                    );
                }
            }
        }
    }

    // Check .env file
    let env_file = project.dir().join(".env");
    if env_file.exists() {
        result.add_success("Environment file: .env".to_string());
    } else {
        result.add_info("Environment file: not found".to_string());
    }
}

fn check_workspace_details(workspace: &Workspace, result: &mut DiagnosticResult) {
    let config = workspace.config();
    let project_count = config.projects.len();

    if project_count == 0 {
        result.add_warning(
            "Workspace has no projects".to_string(),
            Some("Run 'de scan' to discover projects or 'de init' to create new ones".to_string()),
        );
    } else {
        result.add_success(format!("Projects: {} registered", project_count));

        // Check if projects still exist
        let mut valid_projects = 0;
        let mut invalid_projects = 0;

        for (project_id, workspace_project) in &config.projects {
            if workspace_project.dir.exists() {
                valid_projects += 1;
            } else {
                invalid_projects += 1;
                result.add_error(
                    format!(
                        "Missing: {} ({})",
                        project_id,
                        workspace_project.dir.display()
                    ),
                    None,
                );
            }
        }

        if invalid_projects > 0 {
            result.add_warning(
                format!("{} project(s) have missing directories", invalid_projects),
                Some("Run 'de update' to clean up workspace configuration".to_string()),
            );
        }

        if valid_projects > 0 && invalid_projects == 0 {
            result.add_success("All project directories found".to_string());
        }

        // Check for task name conflicts
        check_for_conflicts(workspace, result);
    }
}

fn check_for_conflicts(workspace: &Workspace, result: &mut DiagnosticResult) {
    let config = workspace.config();
    let project_names: std::collections::HashSet<_> = config.projects.keys().collect();

    for (project_id, workspace_project) in &config.projects {
        if !workspace_project.dir.exists() {
            continue;
        }

        let project = match Project::from_dir(&workspace_project.dir) {
            Ok(project) => project,
            Err(e) => {
                result.add_error(
                    format!("Failed to load project {}: {}", project_id, e),
                    None,
                );
                continue;
            }
        };

        if let Some(tasks) = project.tasks() {
            for task_name in tasks.keys() {
                if project_names.contains(task_name) {
                    result.add_warning(
                        format!(
                            "Task '{}' in project '{}' conflicts with a project name.",
                            task_name, project_id
                        ),
                        Some(
                            "Consider renaming the task or project to avoid ambiguity.".to_string(),
                        ),
                    );
                }
            }
        }
    }
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

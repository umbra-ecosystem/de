use eyre::eyre;
use std::process::Command;

use crate::{project::Project, workspace::Workspace};

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
        self.details.push(format!("    - {}", message));
    }
}

pub fn doctor() -> eyre::Result<()> {
    println!("de doctor");
    println!("---------");

    // Check system dependencies
    println!("System Dependencies:");
    let system_result = check_system_dependencies();
    for detail in &system_result.details {
        println!("{}", detail);
    }
    println!();

    // Check project configuration
    println!("Project Configuration:");
    let project_result = check_project_configuration();
    for detail in &project_result.details {
        println!("{}", detail);
    }
    println!();

    // Check workspace configuration
    println!("Workspace Configuration:");
    let workspace_result = check_workspace_configuration();
    for detail in &workspace_result.details {
        println!("{}", detail);
    }

    // Calculate totals and print status
    let total_errors = system_result.errors + project_result.errors + workspace_result.errors;
    let total_warnings =
        system_result.warnings + project_result.warnings + workspace_result.warnings;

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

fn check_workspace_configuration() -> DiagnosticResult {
    let mut result = DiagnosticResult::new();

    match Workspace::active() {
        Ok(Some(workspace)) => {
            result.add_success(format!("Active workspace: {}", workspace.config().name));
            check_workspace_details(&workspace, &mut result);
        }
        Ok(None) => {
            result.add_warning(
                "No active workspace found".to_string(),
                Some("Initialize a project or set an active workspace".to_string()),
            );
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

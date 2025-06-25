use eyre::eyre;
use std::process::Command;

use crate::{project::Project, workspace::Workspace};

pub fn doctor() -> eyre::Result<()> {
    println!("🩺 de Doctor - Health Check");
    println!("═══════════════════════════");
    println!();

    let mut issues_found = 0;
    let mut warnings_found = 0;

    // Check Docker installation
    println!("🐳 Checking Docker installation...");
    match check_docker() {
        Ok(version) => println!("   ✅ Docker is available ({})", version),
        Err(e) => {
            println!("   ❌ Docker is not available: {}", e);
            println!("      💡 Install Docker from https://docs.docker.com/get-docker/");
            issues_found += 1;
        }
    }

    // Check Docker Compose installation
    println!("🐙 Checking Docker Compose installation...");
    match check_docker_compose() {
        Ok(version) => println!("   ✅ Docker Compose is available ({})", version),
        Err(e) => {
            println!("   ❌ Docker Compose is not available: {}", e);
            println!(
                "      💡 Install Docker Compose from https://docs.docker.com/compose/install/"
            );
            issues_found += 1;
        }
    }

    // Check if we're in a de project
    println!("📁 Checking current project...");
    match Project::current() {
        Ok(Some(project)) => {
            println!(
                "   ✅ Found de project: {}",
                project.manifest().project().name
            );

            // Check project configuration
            let project_issues = check_project_health(&project);
            issues_found += project_issues.0;
            warnings_found += project_issues.1;
        }
        Ok(None) => {
            println!("   ⚠️  Not in a de project directory");
            println!("      💡 Run 'de init' to initialize a project here");
            warnings_found += 1;
        }
        Err(e) => {
            println!("   ❌ Error checking project: {}", e);
            issues_found += 1;
        }
    }

    // Check workspace configuration
    println!("🏢 Checking workspace configuration...");
    match Workspace::active() {
        Ok(Some(workspace)) => {
            println!("   ✅ Active workspace: {}", workspace.config().name);

            // Check workspace health
            let (workspace_issues, workspace_warnings) = check_workspace_health(&workspace);
            issues_found += workspace_issues;
            warnings_found += workspace_warnings;
        }
        Ok(None) => {
            println!("   ⚠️  No active workspace found");
            println!("      💡 Initialize a project or set an active workspace");
            warnings_found += 1;
        }
        Err(e) => {
            println!("   ❌ Error checking workspace: {}", e);
            issues_found += 1;
        }
    }

    // Summary
    println!();
    println!("📊 Health Check Summary");
    println!("═══════════════════════");

    if issues_found == 0 && warnings_found == 0 {
        println!("🎉 Everything looks healthy! No issues found.");
    } else {
        if issues_found > 0 {
            println!("❌ {} critical issue(s) found", issues_found);
        }
        if warnings_found > 0 {
            println!("⚠️  {} warning(s) found", warnings_found);
        }
        println!();
        println!("💡 Address the issues above to ensure optimal de experience.");
    }

    Ok(())
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

fn check_project_health(project: &Project) -> (u32, u32) {
    let mut issues = 0;
    let mut warnings = 0;

    // Check if project directory exists
    if !project.dir().exists() {
        println!(
            "   ❌ Project directory does not exist: {}",
            project.dir().display()
        );
        issues += 1;
    }

    // Check if de.toml exists and is readable
    if !project.manifest_path().exists() {
        println!("   ❌ Project manifest (de.toml) does not exist");
        issues += 1;
    } else {
        println!("   ✅ Project manifest found");
    }

    // Check Docker Compose file if configured
    match project.docker_compose_path() {
        Ok(Some(compose_path)) => {
            println!(
                "   ✅ Docker Compose file found: {}",
                compose_path.display()
            );

            // Try to validate the compose file
            if let Err(e) = validate_docker_compose(&compose_path) {
                println!("   ❌ Docker Compose file validation failed: {}", e);
                issues += 1;
            } else {
                println!("   ✅ Docker Compose file is valid");
            }
        }
        Ok(None) => {
            println!("   ℹ️  No Docker Compose file configured (optional)");
        }
        Err(e) => {
            println!("   ❌ Error checking Docker Compose file: {}", e);
            issues += 1;
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
        println!("   ⚠️  No tasks defined in project");
        println!("      💡 Add tasks to your de.toml to make the most of de");
        warnings += 1;
    } else {
        println!("   ✅ {} task(s) defined", task_count);
    }

    // Check .env file
    let env_file = project.dir().join(".env");
    if env_file.exists() {
        println!("   ✅ Environment file (.env) found");
    } else {
        println!("   ℹ️  No .env file found (optional)");
    }

    (issues, warnings)
}

fn check_workspace_health(workspace: &Workspace) -> (u32, u32) {
    let issues = 0;
    let mut warnings = 0;

    let config = workspace.config();
    let project_count = config.projects.len();

    if project_count == 0 {
        println!("   ⚠️  Workspace has no projects");
        println!("      💡 Run 'de scan' to discover projects or 'de init' to create new ones");
        warnings += 1;
    } else {
        println!("   ✅ Workspace contains {} project(s)", project_count);

        // Check if projects still exist
        let mut valid_projects = 0;
        let mut invalid_projects = 0;

        for (project_id, workspace_project) in &config.projects {
            if workspace_project.dir.exists() {
                valid_projects += 1;
            } else {
                invalid_projects += 1;
                println!(
                    "   ❌ Project '{}' directory not found: {}",
                    project_id,
                    workspace_project.dir.display()
                );
            }
        }

        if invalid_projects > 0 {
            println!(
                "   ⚠️  {} project(s) have missing directories",
                invalid_projects
            );
            println!("      💡 Run 'de update' to clean up workspace configuration");
            warnings += 1;
        }

        if valid_projects > 0 {
            println!("   ✅ {} project(s) have valid directories", valid_projects);
        }
    }

    (issues, warnings)
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

use eyre::{WrapErr, eyre};

use crate::{project::Project, utils::theme::Theme, workspace::Workspace};

pub fn list() -> eyre::Result<()> {
    let mut found_tasks = false;
    let theme = Theme::new();

    // Try to get current project (with inference support)
    let project_opt = Project::current_or_inferred()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?;

    if let Some(project) = project_opt.as_ref() {
        // List configured tasks from de.toml
        if let Some(tasks) = project.manifest().tasks.as_ref() {
            if !tasks.is_empty() {
                println!(
                    "{} {}",
                    theme.bold("Configured tasks in project:"),
                    theme.highlight(project.manifest().project().name.as_str())
                );
                for (name, task) in tasks {
                    println!(
                        "  {} {}",
                        theme.accent(name.as_str()),
                        theme.dim(&format!("({})", task.command_str()))
                    );
                }
                found_tasks = true;
            }
        }

        // Detect tasks from project files
        match project.detect_tasks() {
            Ok(detected_tasks) => {
                if !detected_tasks.is_empty() {
                    if found_tasks {
                        println!(); // Add separation if we already printed configured tasks
                    }

                    println!(
                        "{} {}",
                        theme.bold("Detected tasks in project:"),
                        theme.highlight(project.manifest().project().name.as_str())
                    );

                    // Group tasks by source
                    let mut tasks_by_source = std::collections::BTreeMap::new();
                    for (name, task) in detected_tasks {
                        let source_name = task.source.display_name().to_string();
                        tasks_by_source
                            .entry(source_name)
                            .or_insert_with(Vec::new)
                            .push((name, task));
                    }

                    for (source, tasks) in tasks_by_source {
                        println!("  {} {}", theme.dim("from"), theme.accent(&source));
                        for (name, task) in tasks {
                            let description = task
                                .description
                                .map(|d| format!(" - {}", d))
                                .unwrap_or_default();
                            println!(
                                "    {} {}{}",
                                theme.highlight(&name),
                                theme.dim(&format!("({})", task.command)),
                                theme.dim(&description)
                            );
                        }
                    }

                    found_tasks = true;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to detect tasks: {}", e);
            }
        }
    }

    // List workspace tasks
    if let Some(workspace) = Workspace::active()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get active workspace")?
        && !workspace.config().tasks.is_empty()
    {
        if found_tasks {
            println!(); // Add a newline for separation if project tasks were listed
        }
        println!(
            "{} {}",
            theme.bold("Tasks in workspace:"),
            theme.highlight(&workspace.config().name.as_str())
        );
        for (name, command) in &workspace.config().tasks {
            println!(
                "  {} {}",
                theme.accent(name.as_str()),
                theme.dim(&format!("({})", command))
            );
        }
        found_tasks = true;
    }

    if !found_tasks {
        println!("No tasks found in the current project or active workspace.");
        println!();
        println!("Tip: de can auto-detect tasks from:");
        println!("  - package.json (npm scripts)");
        println!("  - Makefile (make targets)");
        println!("  - justfile (just recipes)");
        println!("  - Cargo.toml (cargo commands)");
        println!("  - pyproject.toml (poetry/poe scripts)");
    }

    Ok(())
}

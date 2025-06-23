use eyre::{Context, eyre};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::project::Project;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum Task {
    Compose { service: String, command: String },
    Raw(RawTask),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum RawTask {
    Flat(String),
    Complex { command: String },
}

impl RawTask {
    pub fn command_str(&self) -> &str {
        match self {
            RawTask::Flat(cmd) => cmd,
            RawTask::Complex { command } => command,
        }
    }
}

impl Task {
    pub fn command_str(&self) -> String {
        match self {
            Task::Compose { service, command } => {
                format!("docker-compose exec {} {}", service, command)
            }
            Task::Raw(shell_task) => shell_task.command_str().to_string(),
        }
    }

    pub fn command(&self, project: &Project) -> eyre::Result<Command> {
        match self {
            Task::Compose { service, command } => {
                let mut cmd = Command::new("docker-compose");

                let docker_compose_path = project
                    .docker_compose_path()
                    .map_err(|e| eyre!(e))
                    .wrap_err("Failed to get docker compose path")?
                    .ok_or_else(|| eyre!("Docker compose path not found"))?;

                let docker_compose_path = docker_compose_path
                    .to_str()
                    .ok_or_else(|| eyre!("Invalid docker compose path"))?;

                cmd.arg("-f")
                    .arg(docker_compose_path)
                    .arg("exec")
                    .arg(service);

                let args = command.split_whitespace();
                for arg in args {
                    cmd.arg(arg);
                }

                Ok(cmd)
            }
            Task::Raw(shell_task) => {
                let mut parts = shell_task.command_str().split_whitespace();
                let program = parts.next().ok_or_else(|| eyre!("Empty command"))?;
                let args = parts.collect::<Vec<_>>();

                let dir = project
                    .manifest_path()
                    .parent()
                    .ok_or_else(|| eyre!("Failed to get project directory"))?;

                let mut cmd = Command::new(program);
                cmd.current_dir(dir);
                cmd.args(&args);
                Ok(cmd)
            }
        }
    }
}

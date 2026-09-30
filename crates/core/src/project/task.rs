use eyre::eyre;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::project::Project;

/// A task defined in `de.toml`, either as a bare command string or as a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Task {
    Flat(String),
    Complex { command: String },
}

impl Task {
    pub fn command_str(&self) -> &str {
        match self {
            Task::Flat(cmd) => cmd,
            Task::Complex { command } => command,
        }
    }

    pub fn command(&self, project: &Project) -> eyre::Result<Command> {
        let mut parts = self.command_str().split_whitespace();
        let program = parts.next().ok_or_else(|| eyre!("Empty command"))?;
        let args = parts.collect::<Vec<_>>();

        let mut cmd = Command::new(program);
        cmd.current_dir(project.dir());
        cmd.args(&args);
        Ok(cmd)
    }
}

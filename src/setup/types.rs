use serde::{Deserialize, Serialize};

use crate::setup::project::GitOverride;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitConfig {
    pub url: String,
    #[serde(default)]
    pub branch: Option<String>,
}

impl GitConfig {
    pub fn apply_override(self, git_override: GitOverride) -> Self {
        Self {
            url: git_override.url.unwrap_or(self.url),
            branch: git_override.branch.or(self.branch),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StepService {
    pub name: String,
    pub compose: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApplyCommand {
    command: String,
    #[serde(default)]
    stdin: Option<CommandPipe>,
}

impl From<String> for ApplyCommand {
    fn from(command: String) -> Self {
        Self {
            command,
            stdin: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExportCommand {
    pub command: String,
    #[serde(default)]
    pub stdout: Option<CommandPipe>,
}

impl From<String> for ExportCommand {
    fn from(command: String) -> Self {
        Self {
            command,
            stdout: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum CommandPipe {
    File { file: String },
}

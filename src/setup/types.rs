use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::setup::{project::GitOverride, utils::EnvMapper};

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
    pub command: String,
    #[serde(default)]
    pub stdin: Option<CommandPipe>,
}

impl Display for ApplyCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.command)?;
        if let Some(pipe) = &self.stdin {
            match pipe {
                CommandPipe::File { file } => write!(f, " < {}", file)?,
            }
        }
        Ok(())
    }
}

impl From<String> for ApplyCommand {
    fn from(command: String) -> Self {
        Self {
            command,
            stdin: None,
        }
    }
}

impl ApplyCommand {
    pub fn resolve_env(&self, env_mapper: &EnvMapper) -> Self {
        Self {
            command: env_mapper.format_str(&self.command),
            stdin: self.stdin.as_ref().map(|pipe| match pipe {
                CommandPipe::File { file } => CommandPipe::File {
                    file: env_mapper.format_str(file),
                },
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum CommandPipe {
    File { file: String },
}

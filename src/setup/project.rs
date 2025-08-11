use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::{
    types::Slug,
    utils::serde::{OneOrMany, StringOr},
};

use super::{export::ExportCommand, types::GitConfig};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetupConfig {
    pub git: StringOr<GitConfig>,
    #[serde(default)]
    pub steps: HashMap<Slug, Step>,
    #[serde(default)]
    pub profiles: HashMap<Slug, Profile>,
}

impl From<String> for GitConfig {
    fn from(url: String) -> Self {
        Self { url, branch: None }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    #[serde(default)]
    pub git: Option<StringOr<GitOverride>>,
    #[serde(default)]
    pub steps: HashMap<String, Step>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitOverride {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

impl From<String> for GitOverride {
    fn from(url: String) -> Self {
        Self {
            url: Some(url),
            branch: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Step {
    pub name: String,
    #[serde(default)]
    pub service: Option<StringOr<StepService>>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub skip_if: Option<String>,
    #[serde(flatten)]
    pub kind: StepKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, rename_all = "snake_case")]
pub enum StepKind {
    Standard(StandardStep),
    Complex {
        apply: OneOrMany<StringOr<ApplyCommand>>,
        export: OneOrMany<StringOr<ExportCommand>>,
        #[serde(default)]
        env: Option<BTreeMap<String, String>>,
    },
    Basic {
        command: StringOr<ApplyCommand>,
        #[serde(default)]
        env: Option<BTreeMap<String, String>>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StandardStep {
    CopyFiles {
        source: String,
        #[serde(default)]
        destination: String,
        #[serde(default)]
        overwrite: bool,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StepService {
    pub name: String,
    pub compose: Option<String>,
}

impl From<String> for StepService {
    fn from(name: String) -> Self {
        Self {
            name,
            compose: None,
        }
    }
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
#[serde(untagged, rename_all = "snake_case")]
pub enum CommandPipe {
    File { file: String },
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{
    types::Slug,
    utils::serde::{OneOrMany, StringOr},
};

use super::{export::ExportCommand, types::GitConfig};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetupConfig {
    pub git: StringOr<GitConfig>,
    #[serde(default)]
    pub steps: BTreeMap<Slug, Step>,
    #[serde(default)]
    pub profiles: BTreeMap<Slug, Profile>,
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
    pub steps: BTreeMap<Slug, Step>,
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
        command: OneOrMany<StringOr<ApplyCommand>>,
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

impl SetupConfig {
    pub fn steps(&self, profile: &Slug) -> BTreeMap<Slug, Step> {
        let mut output = self.steps.clone();
        if let Some(profile_steps) = self.profiles.get(profile) {
            for (slug, step) in &profile_steps.steps {
                output.insert(slug.clone(), step.clone());
            }
        }
        output
    }

    pub fn git(&self, profile: &Slug) -> GitConfig {
        let mut git_config = self.git.clone_value();
        if let Some(profile) = self.profiles.get(profile)
            && let Some(git_override) = profile.git.as_ref() {
                git_config = git_config.apply_override(git_override.clone_value());
            }
        git_config
    }
}

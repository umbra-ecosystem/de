use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    types::Slug,
    utils::serde::{OneOrMany, StringOr},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetupConfig {
    pub git: StringOr<GitConfig>,
    #[serde(default)]
    pub steps: HashMap<Slug, Step>,
    #[serde(default)]
    pub profiles: HashMap<Slug, Profile>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitConfig {
    pub url: String,
    #[serde(default)]
    pub branch: Option<String>,
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
    pub r#type: Option<String>,
    #[serde(default)]
    pub service: Option<Service>,
    #[serde(default)]
    pub optional: Option<bool>,
    #[serde(default)]
    pub skip_if: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum StepType {
    CopyFiles {
        source: String,
        #[serde(default)]
        destination: String,
        #[serde(default)]
        overwrite: bool,
    },
    Custom {
        #[serde(default)]
        command: Option<OneOrMany<String>>,
        #[serde(default)]
        env: Option<HashMap<String, String>>,
        #[serde(default)]
        export: Option<Vec<StringOr<Snapshot>>>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Service {
    pub name: String,
    pub compose: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Snapshot {
    pub command: String,
    pub files: Vec<String>,
}

impl From<String> for Snapshot {
    fn from(command: String) -> Self {
        Self {
            command,
            files: vec![],
        }
    }
}

use eyre::{Context, eyre};
use serde::{Deserialize, Serialize};

use crate::{types::Slug, utils::get_project_dirs};

/// Global configuration for the application.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// The active workspace configuration.
    pub active: Option<ActiveConfig>,
    /// Jira settings (`[jira]`). Absent until the user configures Jira.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira: Option<JiraConfig>,
    /// Bitbucket settings (`[bitbucket]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitbucket: Option<BitbucketConfig>,
    /// Pipeline settings (`[pipelines]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipelines: Option<PipelinesConfig>,
}

/// `[jira]` in the global config. Everything is optional so existing files keep loading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JiraConfig {
    /// The Jira Cloud site, e.g. `acme.atlassian.net`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// The author's Jira account id; comments mentioning it are the "you were tagged" signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// The JQL selecting the pool of tickets in the board's Review column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_jql: Option<String>,
    /// The JQL selecting tickets in the Returned status. Derived from `statuses.returned`
    /// when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_jql: Option<String>,
    /// Names of the workflow statuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statuses: Option<JiraStatuses>,
}

/// `[jira.statuses]`: the workflow status names of this Jira project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JiraStatuses {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_testing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned: Option<String>,
    /// Every status that means finished.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
}

impl JiraStatuses {
    pub const DEFAULT_REVIEW: &'static str = "In Review";
    pub const DEFAULT_ALPHA_TESTING: &'static str = "Alpha Testing";
    pub const DEFAULT_UAT: &'static str = "UAT";
    pub const DEFAULT_RETURNED: &'static str = "Returned";
    pub const DEFAULT_DONE: &'static str = "Done";

    /// The Review status name, or the default when unset.
    pub fn review_name(&self) -> &str {
        self.review.as_deref().unwrap_or(Self::DEFAULT_REVIEW)
    }

    pub fn alpha_testing_name(&self) -> &str {
        self.alpha_testing
            .as_deref()
            .unwrap_or(Self::DEFAULT_ALPHA_TESTING)
    }

    pub fn uat_name(&self) -> &str {
        self.uat.as_deref().unwrap_or(Self::DEFAULT_UAT)
    }

    pub fn returned_name(&self) -> &str {
        self.returned.as_deref().unwrap_or(Self::DEFAULT_RETURNED)
    }

    /// The statuses that mean finished; `["Done"]` when unset.
    pub fn done_names(&self) -> Vec<&str> {
        if self.done.is_empty() {
            vec![Self::DEFAULT_DONE]
        } else {
            self.done.iter().map(String::as_str).collect()
        }
    }
}

/// `[bitbucket]` in the global config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitbucketConfig {
    /// The Bitbucket workspace slug; the first half of every `workspace/slug` repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// `[pipelines]` in the global config (a project's `[pipelines]` overrides it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelinesConfig {
    /// The deployment environment that counts as "deployed". Unset: any step that has a
    /// deployment environment counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy_environment: Option<String>,
}

/// Configuration for the active workspace.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ActiveConfig {
    pub workspace: Option<Slug>,
}

impl Config {
    pub fn config_path() -> eyre::Result<std::path::PathBuf> {
        let project_dirs = get_project_dirs()?;
        Ok(project_dirs.config_dir().join("config.toml"))
    }

    pub fn save(&self) -> eyre::Result<()> {
        let config_path = Self::config_path()?;

        let config_str = toml::to_string_pretty(self)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to format config as string")?;

        std::fs::write(&config_path, config_str)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to write config to {}", config_path.display()))?;

        Ok(())
    }

    pub fn load() -> eyre::Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path)
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| {
                    format!("Failed to read config file at {}", config_path.display())
                })?;
            Self::parse(&config_str)
        } else {
            Ok(Self::default())
        }
    }
}

impl Config {
    /// Parses the text of a config file.
    pub fn parse(text: &str) -> eyre::Result<Self> {
        toml::from_str(text)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to parse config file")
    }

    /// The `[jira]` section, or an empty one.
    pub fn jira(&self) -> JiraConfig {
        self.jira.clone().unwrap_or_default()
    }

    /// The Jira workflow status names; use the `*_name` accessors to get defaults filled in.
    pub fn jira_statuses(&self) -> JiraStatuses {
        self.jira
            .as_ref()
            .and_then(|j| j.statuses.clone())
            .unwrap_or_default()
    }

    /// The author's Jira account id, if configured.
    pub fn jira_account_id(&self) -> Option<&str> {
        self.jira.as_ref().and_then(|j| j.account_id.as_deref())
    }

    /// The Bitbucket workspace slug, if configured.
    pub fn bitbucket_workspace(&self) -> Option<&str> {
        self.bitbucket.as_ref().and_then(|b| b.workspace.as_deref())
    }

    /// The globally configured deployment environment, if any.
    pub fn deploy_environment(&self) -> Option<&str> {
        self.pipelines
            .as_ref()
            .and_then(|p| p.deploy_environment.as_deref())
    }
}

impl Config {
    /// Loads the current configuration, applies the provided mutation function, and saves the modified configuration.
    pub fn mutate_persisted<F>(f: F) -> eyre::Result<Config>
    where
        F: FnOnce(&mut Config),
    {
        let mut config = Self::load()?;
        f(&mut config);
        config
            .save()
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to save modified workspace configuration")?;
        Ok(config)
    }
}

impl Config {
    pub fn get_active_workspace(&self) -> Option<&Slug> {
        self.active
            .as_ref()
            .and_then(|active| active.workspace.as_ref())
    }

    pub fn set_active_workspace(&mut self, workspace: Option<Slug>) {
        if let Some(active) = &mut self.active {
            active.workspace = workspace;
        } else {
            self.active = Some(ActiveConfig { workspace });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_config_still_parses() {
        let c = Config::parse("[active]\nworkspace = \"shop\"\n").unwrap();
        assert_eq!(c.get_active_workspace().unwrap().as_str(), "shop");
        assert!(c.jira.is_none() && c.bitbucket.is_none() && c.pipelines.is_none());
        assert!(Config::parse("").unwrap().jira.is_none());
    }

    #[test]
    fn full_provider_config_parses() {
        let c = Config::parse(
            r#"
            [jira]
            site = "acme.atlassian.net"
            account_id = "abc123"
            review_jql = "project = PROJ AND status = 'In Review'"
            [jira.statuses]
            review = "Code Review"
            alpha_testing = "Alpha"
            uat = "UAT"
            returned = "Sent Back"
            done = ["Done", "Closed"]
            [bitbucket]
            workspace = "acme"
            [pipelines]
            deploy_environment = "alpha"
            "#,
        )
        .unwrap();
        assert_eq!(c.jira_account_id(), Some("abc123"));
        assert_eq!(c.jira().site.as_deref(), Some("acme.atlassian.net"));
        let s = c.jira_statuses();
        assert_eq!(s.review_name(), "Code Review");
        assert_eq!(s.alpha_testing_name(), "Alpha");
        assert_eq!(s.returned_name(), "Sent Back");
        assert_eq!(s.done_names(), ["Done", "Closed"]);
        assert_eq!(c.bitbucket_workspace(), Some("acme"));
        assert_eq!(c.deploy_environment(), Some("alpha"));
    }

    #[test]
    fn defaults_apply_when_sections_or_keys_are_missing() {
        let c = Config::parse("[jira]\nsite = \"x\"\n").unwrap();
        let s = c.jira_statuses();
        assert_eq!(s.review_name(), "In Review");
        assert_eq!(s.alpha_testing_name(), "Alpha Testing");
        assert_eq!(s.uat_name(), "UAT");
        assert_eq!(s.returned_name(), "Returned");
        assert_eq!(s.done_names(), ["Done"]);
        assert_eq!(c.bitbucket_workspace(), None);
        assert_eq!(c.deploy_environment(), None);
        assert_eq!(Config::default().jira_account_id(), None);
    }

    #[test]
    fn saving_does_not_add_empty_sections() {
        let saved = toml::to_string_pretty(&Config::default()).unwrap();
        for section in ["jira", "bitbucket", "pipelines"] {
            assert!(!saved.contains(section), "{saved}");
        }
        let with = Config::parse("[jira]\naccount_id = \"a\"\n").unwrap();
        let again = Config::parse(&toml::to_string_pretty(&with).unwrap()).unwrap();
        assert_eq!(again.jira, with.jira);
    }
}

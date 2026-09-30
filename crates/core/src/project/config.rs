use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    str::FromStr,
};

use eyre::{Context, eyre};
use serde::{Deserialize, Serialize};

use crate::{domain::BaselineChoice, git::GitRepo, project::task::Task, types::Slug};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectManifest {
    #[serde(default)]
    pub project: ProjectMetadata,
    #[serde(default)]
    pub git: Option<ProjectGitSettings>,
    #[serde(default)]
    pub tasks: Option<BTreeMap<Slug, Task>>,
    /// Branch names this repo uses (`[branches]`).
    #[serde(default, skip_serializing_if = "BranchesConfig::is_empty")]
    pub branches: BranchesConfig,
    /// Test overlay this repo consumes (`[overlay]`).
    #[serde(default, skip_serializing_if = "OverlayConfig::is_empty")]
    pub overlay: OverlayConfig,
    /// Tasks to run when a ticket branch is activated (`[activate]`).
    #[serde(default, skip_serializing_if = "ActivateConfig::is_empty")]
    pub activate: ActivateConfig,
    /// Where this repo is hosted (`[hosting]`); repos without it are never synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosting: Option<HostingConfig>,
    /// Pipeline settings overriding the global ones (`[pipelines]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipelines: Option<ProjectPipelinesConfig>,
}

impl ProjectManifest {
    /// The `workspace/slug` this project is hosted at, or `None` when it has no `[hosting]`.
    ///
    /// An explicit `repo` wins. Otherwise it is `<global bitbucket workspace>/<directory
    /// name>`; without a global workspace there is nothing to derive it from.
    pub fn hosting_repo(
        &self,
        dir_name: &str,
        bitbucket_workspace: Option<&str>,
    ) -> Option<String> {
        let hosting = self.hosting.as_ref()?;
        match hosting.repo.as_deref() {
            Some(repo) => Some(repo.into()),
            None => bitbucket_workspace.map(|ws| format!("{ws}/{dir_name}")),
        }
    }

    /// The deployment environment for this project: its own `[pipelines]` value, else `global`.
    pub fn deploy_environment<'a>(&'a self, global: Option<&'a str>) -> Option<&'a str> {
        self.pipelines
            .as_ref()
            .and_then(|p| p.deploy_environment.as_deref())
            .or(global)
    }

    pub fn project(&self) -> &ProjectMetadata {
        &self.project
    }

    pub fn load(path: &Path) -> eyre::Result<ProjectManifest> {
        let manifest_str = std::fs::read_to_string(path)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to read manifest file at {}", path.display()))?;

        toml::from_str(&manifest_str)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to parse project manifest")
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        let manifest_str = toml::to_string_pretty(&self)
            .map_err(|e| eyre!(e))
            .wrap_err("Failed to format manifest as string")?;

        std::fs::write(path, manifest_str)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to write manifest to {}", path.display()))?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    #[serde(default = "default_project_name")]
    pub name: Slug,
    #[serde(default = "default_project_workspace")]
    pub workspace: Slug,
    #[serde(default)]
    pub docker_compose: Option<PathBuf>,
    #[serde(default)]
    pub depends_on: Option<Vec<Slug>>,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            workspace: default_project_workspace(),
            docker_compose: Default::default(),
            depends_on: Default::default(),
        }
    }
}

fn default_project_name() -> Slug {
    Slug::from_str("default").expect("default project name should be valid")
}

fn default_project_workspace() -> Slug {
    Slug::from_str("default").expect("default workspace name should be valid")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectGitSettings {
    #[serde(default = "default_git_enabled")]
    pub enabled: bool,
    #[serde(default = "default_git_remote")]
    pub default_remote: String,
}

impl Default for ProjectGitSettings {
    fn default() -> Self {
        Self {
            enabled: default_git_enabled(),
            default_remote: default_git_remote(),
        }
    }
}

fn default_git_enabled() -> bool {
    true
}

fn default_git_remote() -> String {
    "origin".to_string()
}

/// `[branches]`: which branches this repo uses for the ticket flow. Everything is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchesConfig {
    /// Fallback when a ticket has no branch here (default `develop`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The production branch, `master` or `main`; when unset both are tried, `master` first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production: Option<String>,
    /// The integration branch (default `uat`), pushed to directly and never a PR target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uat: Option<String>,
}

impl BranchesConfig {
    pub const DEFAULT_BASE: &'static str = "develop";
    pub const DEFAULT_UAT: &'static str = "uat";
    /// Tried in this order when `production` is not configured.
    pub const PRODUCTION_FALLBACKS: [&'static str; 2] = ["master", "main"];

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Branch names to look for, best first. An explicit setting is the only candidate. The
    /// base falls back to the workspace's `default_branch`, then `develop`.
    pub fn candidates<'a>(
        &'a self,
        choice: BaselineChoice,
        workspace_default: Option<&'a str>,
    ) -> Vec<&'a str> {
        match choice {
            BaselineChoice::Base => match self.base.as_deref() {
                Some(base) => vec![base],
                None => {
                    let mut names: Vec<&str> = workspace_default.into_iter().collect();
                    if !names.contains(&Self::DEFAULT_BASE) {
                        names.push(Self::DEFAULT_BASE);
                    }
                    names
                }
            },
            BaselineChoice::Production => match self.production.as_deref() {
                Some(production) => vec![production],
                None => Self::PRODUCTION_FALLBACKS.to_vec(),
            },
            BaselineChoice::Uat => vec![self.uat.as_deref().unwrap_or(Self::DEFAULT_UAT)],
        }
    }

    /// The baseline branch that actually exists in `repo` (locally or on a remote), or `None`.
    pub fn resolve(
        &self,
        repo: &GitRepo,
        choice: BaselineChoice,
        workspace_default: Option<&str>,
    ) -> eyre::Result<Option<String>> {
        repo.default_branch(&self.candidates(choice, workspace_default))
    }
}

/// The kinds of code host a project can declare.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostingProvider {
    #[default]
    Bitbucket,
}

/// `[hosting]`: the code host of this project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostingConfig {
    /// The provider; `bitbucket` when unset.
    #[serde(default)]
    pub provider: HostingProvider,
    /// `workspace/slug`. Derived from the global Bitbucket workspace and the project
    /// directory name when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// A project's `[pipelines]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPipelinesConfig {
    /// Overrides the global `deploy_environment` for this project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy_environment: Option<String>,
}

/// `[overlay]`. Only repos that consume a package built from another workspace repo have one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composer: Option<ComposerOverlayConfig>,
}

impl OverlayConfig {
    pub fn is_empty(&self) -> bool {
        self.composer.is_none()
    }
}

/// `[overlay.composer]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerOverlayConfig {
    /// Composer package name -> the workspace project that provides it.
    #[serde(default)]
    pub packages: BTreeMap<String, String>,
    /// `de` task names run, in order, after the overlay is applied.
    #[serde(default)]
    pub rebuild: Vec<String>,
}

/// `[activate]`: rebuilds for repos that need one whenever they sit on a ticket branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateConfig {
    #[serde(default)]
    pub after: Vec<String>,
}

impl ActivateConfig {
    pub fn is_empty(&self) -> bool {
        self.after.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_str: &str) -> ProjectManifest {
        toml::from_str(toml_str).expect("manifest parses")
    }

    #[test]
    fn full_config_parses() {
        let m = parse(
            r#"
            [project]
            name = "web"
            workspace = "shop"

            [tasks]
            build-ui = "npm run build"

            [branches]
            base = "main"
            production = "main"
            uat = "integration"

            [overlay.composer]
            packages = { "acme/api-client" = "api-client", "acme/ui-kit" = "ui" }
            rebuild = ["build-ui", "warm"]

            [activate]
            after = ["build-ui"]
            "#,
        );

        assert_eq!(m.branches.base.as_deref(), Some("main"));
        assert_eq!(m.branches.production.as_deref(), Some("main"));
        assert_eq!(m.branches.uat.as_deref(), Some("integration"));
        let composer = m.overlay.composer.expect("composer overlay");
        assert_eq!(composer.packages.len(), 2);
        assert_eq!(composer.packages["acme/api-client"], "api-client");
        assert_eq!(composer.rebuild, ["build-ui", "warm"]);
        assert_eq!(m.activate.after, ["build-ui"]);
    }

    #[test]
    fn partial_config_leaves_the_rest_unset() {
        let m = parse("[branches]\nbase = \"trunk\"\n");
        assert_eq!(m.branches.base.as_deref(), Some("trunk"));
        assert_eq!(m.branches.production, None);
        assert_eq!(m.branches.uat, None);
        assert!(m.overlay.is_empty());
        assert!(m.activate.is_empty());

        // An overlay without rebuild steps, and one without packages.
        let m = parse("[overlay.composer]\npackages = { \"a/b\" = \"b\" }\n");
        assert!(m.overlay.composer.unwrap().rebuild.is_empty());
        let m = parse("[overlay.composer]\nrebuild = [\"x\"]\n");
        assert!(m.overlay.composer.unwrap().packages.is_empty());
    }

    #[test]
    fn empty_and_pre_existing_manifests_still_parse() {
        let m = parse("");
        assert!(m.branches.is_empty() && m.overlay.is_empty() && m.activate.is_empty());

        let m = parse(
            "[project]\nname = \"api\"\nworkspace = \"shop\"\ndepends_on = [\"db\"]\n[git]\nenabled = false\n",
        );
        assert_eq!(m.project.name.as_str(), "api");
        assert!(m.branches.is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored_like_everywhere_else() {
        let m = parse(
            "future = 1\n[branches]\nbase = \"x\"\nmystery = true\n[overlay.npm]\nfoo = 1\n[activate]\nafter = []\nlater = 2\n",
        );
        assert_eq!(m.branches.base.as_deref(), Some("x"));
        assert!(m.overlay.composer.is_none());
    }

    #[test]
    fn saving_does_not_add_empty_sections() {
        let saved = toml::to_string_pretty(&parse("[project]\nname = \"api\"\n")).unwrap();
        for section in ["branches", "overlay", "activate"] {
            assert!(!saved.contains(section), "{saved}");
        }

        let with = parse(
            "[branches]\nbase = \"trunk\"\n[overlay.composer]\npackages = { \"a/b\" = \"b\" }\n",
        );
        let round_trip = parse(&toml::to_string_pretty(&with).unwrap());
        assert_eq!(round_trip.branches, with.branches);
        assert_eq!(round_trip.overlay, with.overlay);
    }

    #[test]
    fn hosting_and_pipelines_parse_with_defaults() {
        let m = parse(
            "[hosting]\nprovider = \"bitbucket\"\nrepo = \"acme/web\"\n[pipelines]\ndeploy_environment = \"alpha\"\n",
        );
        assert_eq!(
            m.hosting_repo("web-dir", Some("other")).as_deref(),
            Some("acme/web")
        );
        assert_eq!(m.deploy_environment(Some("global")), Some("alpha"));

        // Bare `[hosting]` derives the repo from the global workspace and directory name.
        let m = parse("[hosting]\n");
        assert_eq!(
            m.hosting.as_ref().unwrap().provider,
            HostingProvider::Bitbucket
        );
        assert_eq!(
            m.hosting_repo("web", Some("acme")).as_deref(),
            Some("acme/web")
        );
        assert_eq!(m.hosting_repo("web", None), None);
        assert_eq!(m.deploy_environment(Some("global")), Some("global"));
        assert_eq!(m.deploy_environment(None), None);

        // No `[hosting]`: never hosted, whatever the global config says.
        assert_eq!(parse("").hosting_repo("web", Some("acme")), None);
        assert!(toml::from_str::<ProjectManifest>("[hosting]\nprovider = \"nope\"\n").is_err());
    }

    #[test]
    fn saving_does_not_add_hosting_when_absent() {
        let saved = toml::to_string_pretty(&parse("[project]\nname = \"api\"\n")).unwrap();
        assert!(
            !saved.contains("hosting") && !saved.contains("pipelines"),
            "{saved}"
        );
    }

    #[test]
    fn candidate_defaults() {
        let none = BranchesConfig::default();
        assert_eq!(none.candidates(BaselineChoice::Base, None), ["develop"]);
        assert_eq!(
            none.candidates(BaselineChoice::Base, Some("trunk")),
            ["trunk", "develop"]
        );
        assert_eq!(
            none.candidates(BaselineChoice::Base, Some("develop")),
            ["develop"]
        );
        assert_eq!(
            none.candidates(BaselineChoice::Production, None),
            ["master", "main"]
        );
        assert_eq!(none.candidates(BaselineChoice::Uat, None), ["uat"]);

        let set = BranchesConfig {
            base: Some("dev".into()),
            production: Some("main".into()),
            uat: Some("staging".into()),
        };
        // Explicit settings win over the workspace default and are the only candidate.
        assert_eq!(set.candidates(BaselineChoice::Base, Some("trunk")), ["dev"]);
        assert_eq!(set.candidates(BaselineChoice::Production, None), ["main"]);
        assert_eq!(set.candidates(BaselineChoice::Uat, None), ["staging"]);
    }
}

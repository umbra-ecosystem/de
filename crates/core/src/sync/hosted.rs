//! Which workspace repos are hosted where.

use std::path::PathBuf;

use crate::activation::WorkspaceRepo;
use crate::config::Config;
use crate::project::config::BranchesConfig;

/// A workspace repo that has a `[hosting]` section, with everything sync and the kind rule
/// need to know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedRepo {
    /// The workspace project name (how tickets link to it and how `uat_merges` name it).
    pub project: String,
    /// The code-host path, `workspace/slug`.
    pub repo: String,
    pub dir: PathBuf,
    /// The repo's `[branches]` (the production branch decides hotfixes).
    pub branches: BranchesConfig,
    /// The deployment environment that counts as deployed for this repo: the project's
    /// `[pipelines]` value, else the global one, else `None` (any deployment step).
    pub deploy_environment: Option<String>,
}

impl HostedRepo {
    /// The hosted repos among `repos`. Projects without `[hosting]`, or whose repo path
    /// cannot be determined (no explicit `repo` and no global Bitbucket workspace), are left
    /// out. Order follows `repos`.
    pub fn from_workspace_repos(repos: &[WorkspaceRepo], config: &Config) -> Vec<HostedRepo> {
        repos
            .iter()
            .filter_map(|r| {
                let dir_name = r.dir.file_name()?.to_string_lossy().into_owned();
                let repo = r
                    .manifest
                    .hosting_repo(&dir_name, config.bitbucket_workspace())?;
                Some(HostedRepo {
                    project: r.name.clone(),
                    repo,
                    dir: r.dir.clone(),
                    branches: r.manifest.branches.clone(),
                    deploy_environment: r
                        .manifest
                        .deploy_environment(config.deploy_environment())
                        .map(String::from),
                })
            })
            .collect()
    }

    /// The hosted repo whose code-host path is `repo` (case-insensitive).
    pub fn find<'a>(hosted: &'a [HostedRepo], repo: &str) -> Option<&'a HostedRepo> {
        hosted.iter().find(|h| h.repo.eq_ignore_ascii_case(repo))
    }

    /// Whether `branch` is this repo's production branch: the configured one, else
    /// `master` or `main`.
    pub fn is_production_branch(&self, branch: &str) -> bool {
        is_production_branch(Some(&self.branches), branch)
    }
}

/// Whether `branch` is a production branch given a repo's `[branches]` (`None`: unknown repo).
pub fn is_production_branch(branches: Option<&BranchesConfig>, branch: &str) -> bool {
    match branches.and_then(|b| b.production.as_deref()) {
        Some(configured) => configured == branch,
        None => BranchesConfig::PRODUCTION_FALLBACKS.contains(&branch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::config::ProjectManifest;

    fn workspace_repo(name: &str, dir: &str, manifest: &str) -> WorkspaceRepo {
        WorkspaceRepo {
            name: name.into(),
            dir: PathBuf::from(dir),
            manifest: toml::from_str::<ProjectManifest>(manifest).unwrap(),
        }
    }

    #[test]
    fn only_projects_with_hosting_are_included_and_repo_paths_are_derived() {
        let config = Config::parse(
            "[bitbucket]\nworkspace = \"acme\"\n[pipelines]\ndeploy_environment = \"alpha\"\n",
        )
        .unwrap();
        let repos = [
            workspace_repo("web", "/code/web-app", "[hosting]\n"),
            workspace_repo(
                "api",
                "/code/api",
                "[hosting]\nrepo = \"other/api-svc\"\n[pipelines]\ndeploy_environment = \"uat\"\n[branches]\nproduction = \"main\"\n",
            ),
            workspace_repo("docs", "/code/docs", ""),
        ];

        let hosted = HostedRepo::from_workspace_repos(&repos, &config);
        assert_eq!(hosted.len(), 2);
        assert_eq!(hosted[0].project, "web");
        assert_eq!(hosted[0].repo, "acme/web-app");
        assert_eq!(hosted[0].deploy_environment.as_deref(), Some("alpha"));
        assert_eq!(hosted[1].repo, "other/api-svc");
        assert_eq!(hosted[1].deploy_environment.as_deref(), Some("uat"));
        assert!(hosted[1].is_production_branch("main"));
        assert!(!hosted[1].is_production_branch("master"));
        assert!(HostedRepo::find(&hosted, "ACME/web-app").is_some());
    }

    #[test]
    fn without_a_global_workspace_a_bare_hosting_section_cannot_be_resolved() {
        let repos = [workspace_repo("web", "/code/web", "[hosting]\n")];
        assert!(HostedRepo::from_workspace_repos(&repos, &Config::default()).is_empty());
    }
}

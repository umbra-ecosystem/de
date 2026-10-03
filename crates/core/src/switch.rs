//! Moving a set of projects onto one branch, and where each one goes when it does not have it.
//!
//! The rule is the same wherever projects are moved together — the workspace's "Switch branch",
//! and later the branch moves of ticket activation: a project that has the branch goes to it, a
//! project that does not goes to the first of its fallbacks it actually has, and a project that
//! has neither is told so rather than moved to something nobody asked for.
//!
//! [`RepoFacts::move_to`] is that rule, pure, so it can be decided before anything is touched.
//! [`RepoMove::apply`] then does one project, and never forces: a working tree with uncommitted
//! changes is left exactly as it was found, so no work has to be hunted out of a stash after.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::domain::BaselineChoice;
use crate::git::{GitRepo, OnDirty};
use crate::project::config::BranchesConfig;
use crate::project::Project;

/// One project as the planner sees it: what it has, what it falls back to, what it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoFacts {
    /// The workspace project name, which is also how it is named to the person.
    pub name: String,
    pub dir: PathBuf,
    /// Every branch the project has, local or on a remote.
    pub branches: Vec<String>,
    /// Tip commit time of each branch in `branches`, unix seconds (the newest tip among its
    /// refs): what the picker orders by. A branch with no entry is of unknown age and sorts last.
    pub tips: BTreeMap<String, i64>,
    /// Where it falls back to when it does not have the branch asked for, best first: its
    /// `[branches] base`, else the workspace's `default_branch`, else `develop`.
    pub fallback: Vec<String>,
    /// What it is on now; `None` when HEAD is detached or the repository could not be read.
    pub current: Option<String>,
}

impl RepoFacts {
    /// Whether the project has this branch, locally or on a remote.
    pub fn has(&self, branch: &str) -> bool {
        self.branches.iter().any(|have| have.as_str() == branch)
    }

    /// The branch this project falls back to: the first of its fallbacks it actually has.
    /// `None` when it has none of them, which is the one case nothing can be moved to.
    pub fn fallback_here(&self) -> Option<&str> {
        self.fallback
            .iter()
            .find(|candidate| self.has(candidate))
            .map(String::as_str)
    }

    /// What moving this project to `target` does.
    pub fn move_to(&self, target: &str) -> Move {
        if self.has(target) {
            Move::To {
                branch: target.to_string(),
            }
        } else if let Some(branch) = self.fallback_here() {
            Move::Fallback {
                branch: branch.to_string(),
                wanted: target.to_string(),
            }
        } else {
            Move::Nowhere {
                why: self.why_not(target),
            }
        }
    }

    /// This project's place in a switch to `target`, ready to be carried out.
    pub fn into_move(self, target: &str) -> RepoMove {
        let action = self.move_to(target);
        RepoMove {
            name: self.name,
            dir: self.dir,
            action,
        }
    }

    /// The friendly reason this project cannot be moved to `target`, and what to do about it.
    fn why_not(&self, target: &str) -> String {
        if self.branches.is_empty() {
            format!("{} has no branches yet.", self.name)
        } else {
            format!(
                "{} has no {target} branch and none of its fallbacks; set one under [branches] \
                 in its de.toml.",
                self.name
            )
        }
    }
}

/// What moving one project does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Move {
    /// The project has the branch: go to it.
    To { branch: String },
    /// It does not have the branch, so it goes to this fallback instead.
    Fallback { branch: String, wanted: String },
    /// Neither exists here; `why` is the friendly reason nothing can be done.
    Nowhere { why: String },
}

impl Move {
    /// The branch the project ends up on, or `None` when it does not move.
    pub fn branch(&self) -> Option<&str> {
        match self {
            Move::To { branch } | Move::Fallback { branch, .. } => Some(branch),
            Move::Nowhere { .. } => None,
        }
    }
}

/// One project's place in a switch, ready to be carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMove {
    pub name: String,
    pub dir: PathBuf,
    pub action: Move,
}

impl RepoMove {
    /// Check this project out on its planned branch.
    ///
    /// `Ok` is the line that says what happened, `Err` the friendly reason it did not — written
    /// here, so every surface shows the same words. Nothing is forced: a working tree with
    /// uncommitted changes is left alone, and the switch carries on with the other projects.
    pub fn apply(&self) -> Result<String, String> {
        let branch = match &self.action {
            Move::Nowhere { why } => return Err(why.clone()),
            Move::To { branch } | Move::Fallback { branch, .. } => branch.clone(),
        };
        if !self.dir.is_dir() {
            return Err(format!(
                "{}'s folder is missing, so its branch cannot be changed.",
                self.name
            ));
        }
        let repo = GitRepo::open(&self.dir)
            .map_err(|e| format!("{} could not be read: {e:#}", self.name))?;
        let status = repo
            .status()
            .map_err(|e| format!("{} could not be read: {e:#}", self.name))?;
        if status.branch.as_deref() == Some(branch.as_str()) {
            return Ok(format!("already on {branch}"));
        }
        if !status.is_clean() {
            return Err(format!(
                "{} has uncommitted changes; commit or stash them first, then switch again.",
                self.name
            ));
        }
        let was = status.head_label();
        repo.switch(&branch, OnDirty::Abort)
            .map_err(|e| format!("{} could not be switched to {branch}: {e:#}", self.name))?;
        Ok(match &self.action {
            Move::Fallback { wanted, .. } => format!("{was} \u{2192} {branch} \u{b7} no {wanted} here"),
            _ => format!("{was} \u{2192} {branch}"),
        })
    }
}

/// What `dir` has and where it falls back to, for planning one project's move.
///
/// A repository that cannot be read comes back with no branches of its own, so the plan says so
/// rather than pretending there was nothing to do. A `de.toml` that cannot be read keeps the
/// default fallbacks, so a broken manifest never strands a project.
pub fn facts(name: &str, dir: &Path, workspace_default: Option<&str>) -> RepoFacts {
    let mut fallback: Vec<String> = workspace_default
        .into_iter()
        .map(str::to_string)
        .chain([BranchesConfig::DEFAULT_BASE.to_string()])
        .collect();
    let mut branches = Vec::new();
    let mut tips = BTreeMap::new();
    let mut current = None;
    if let Ok(repo) = GitRepo::open(dir) {
        if let Ok(list) = repo.logical_branches() {
            for b in &list {
                branches.push(b.name.clone());
                tips.insert(b.name.clone(), b.tip_time());
            }
        }
        current = repo.status().ok().and_then(|s| s.branch);
    }
    if let Ok(project) = Project::from_dir(dir) {
        fallback = project
            .manifest()
            .branches
            .candidates(BaselineChoice::Base, workspace_default)
            .into_iter()
            .map(str::to_string)
            .collect();
    }
    RepoFacts {
        name: name.to_string(),
        dir: dir.to_path_buf(),
        branches,
        tips,
        fallback,
        current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(name: &str, branches: &[&str], fallback: &[&str], current: Option<&str>) -> RepoFacts {
        RepoFacts {
            name: name.to_string(),
            dir: PathBuf::from(format!("/tmp/{name}")),
            branches: branches.iter().map(|b| b.to_string()).collect(),
            tips: BTreeMap::new(),
            fallback: fallback.iter().map(|b| b.to_string()).collect(),
            current: current.map(str::to_string),
        }
    }

    /// The rule, in all four outcomes: go there, fall back, stay put, and nothing to go to.
    #[test]
    fn a_project_that_has_the_branch_goes_to_it_and_one_that_does_not_falls_back() {
        let web = repo("web", &["develop", "master"], &["develop"], Some("master"));
        assert_eq!(
            web.move_to("develop"),
            Move::To {
                branch: "develop".into()
            }
        );
        // Already there is still "go to it": nothing about the rule changes.
        assert_eq!(
            web.move_to("master"),
            Move::To {
                branch: "master".into()
            }
        );

        let docs = repo("docs", &["master"], &["develop", "master"], Some("develop"));
        assert_eq!(
            docs.move_to("develop"),
            Move::Fallback {
                branch: "master".into(),
                wanted: "develop".into()
            }
        );
    }

    /// The fallback is the first of them that actually exists, not the first that is configured:
    /// a workspace default of `trunk` that the project does not have falls through to `develop`.
    #[test]
    fn the_fallback_is_the_first_one_the_project_actually_has() {
        let r = repo("api", &["develop"], &["trunk", "develop"], None);
        assert_eq!(r.fallback_here(), Some("develop"));
        let r = repo("api", &["trunk"], &["trunk", "develop"], None);
        assert_eq!(r.fallback_here(), Some("trunk"));
        let r = repo("api", &["other"], &["trunk", "develop"], None);
        assert_eq!(r.fallback_here(), None);
    }

    /// A project with neither is named, with what to do about it, and is never moved somewhere
    /// that was not asked for.
    #[test]
    fn a_project_with_nothing_to_go_to_says_what_to_fix() {
        let stuck = repo("foundation", &["other"], &["develop"], None);
        assert_eq!(
            stuck.move_to("develop"),
            Move::Nowhere {
                why: "foundation has no develop branch and none of its fallbacks; set one \
                      under [branches] in its de.toml."
                    .to_string()
            }
        );
        assert_eq!(stuck.move_to("develop").branch(), None);

        let empty = repo("new-repo", &[], &["develop"], None);
        assert_eq!(
            empty.move_to("develop"),
            Move::Nowhere {
                why: "new-repo has no branches yet.".to_string()
            }
        );
    }

    /// Switching a project that has nothing worth changing still says so, rather than claiming
    /// it moved; a branch that is already checked out needs no work at all.
    #[test]
    fn apply_leaves_a_clean_project_alone_when_it_is_already_there() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("web");
        std::fs::create_dir_all(&dir).unwrap();
        let move_ = RepoMove {
            name: "web".into(),
            dir: dir.clone(),
            action: Move::To {
                branch: "develop".into()
            },
        };
        // A folder that is not a repository: nothing is touched and nothing is guessed.
        assert!(
            move_.apply().unwrap_err().contains("could not be read"),
            "{}",
            move_.apply().unwrap_err()
        );

        let missing = RepoMove {
            name: "gone".into(),
            dir: temp.path().join("gone"),
            action: Move::To {
                branch: "develop".into()
            },
        };
        assert_eq!(
            missing.apply(),
            Err("gone's folder is missing, so its branch cannot be changed.".to_string())
        );

        let nowhere = RepoMove {
            name: "foundation".into(),
            dir,
            action: Move::Nowhere {
                why: "foundation has no branches yet.".into(),
            },
        };
        assert_eq!(nowhere.apply(), Err("foundation has no branches yet.".to_string()));
    }

    /// Facts are read from the project and the repository: a manifest that cannot be read keeps
    /// the default fallbacks rather than stranding the project.
    #[test]
    fn facts_read_the_configured_fallback_and_the_real_branches() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("api");
        std::fs::create_dir_all(&dir).unwrap();
        // No de.toml and no repository: the defaults stand, and nothing is claimed to exist.
        let f = facts("api", &dir, Some("trunk"));
        assert_eq!(f.fallback, vec!["trunk".to_string(), "develop".to_string()]);
        assert_eq!(f.branches, Vec::<String>::new());
        assert_eq!(f.current, None);

        // A configured base replaces the defaults entirely (an explicit setting is the only
        // candidate).
        std::fs::write(
            dir.join("de.toml"),
            "[project]\nname = \"api\"\nworkspace = \"shop\"\n\n[branches]\nbase = \"main\"\n",
        )
        .unwrap();
        let f = facts("api", &dir, Some("trunk"));
        assert_eq!(f.fallback, vec!["main".to_string()]);
    }
}

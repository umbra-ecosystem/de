//! Deciding what activating a ticket will do. Pure: it works on facts that were read
//! beforehand and performs no I/O, so every rule is unit-tested without a repository.
//!
//! A ticket **touches** a repo when a matching branch exists there; touched repos are
//! switched to that branch and every other repo falls back to a baseline branch. The
//! composer overlay is planned independently of that: a consumer gets it whenever a provider
//! it maps is switched to the ticket branch, whatever the consumer itself is doing.

use std::path::PathBuf;

use crate::{
    domain::{BaselineChoice, RepoLinkOrigin, TicketKey},
    overlay::OverlayPackage,
    project::config::ComposerOverlayConfig,
    store::{links::RepoLink, restore::RepoRole},
};

/// The baseline branch each choice resolves to in one repo (`None`: it does not exist there).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baselines {
    pub base: Option<String>,
    pub production: Option<String>,
    pub uat: Option<String>,
}

impl Baselines {
    pub fn get(&self, choice: BaselineChoice) -> Option<&str> {
        match choice {
            BaselineChoice::Base => self.base.as_deref(),
            BaselineChoice::Production => self.production.as_deref(),
            BaselineChoice::Uat => self.uat.as_deref(),
        }
    }
}

/// What the planner needs to know about one workspace repo.
#[derive(Debug, Clone, Default)]
pub struct PlanRepo {
    /// Workspace project name.
    pub name: String,
    pub dir: PathBuf,
    /// Branches whose name contains the ticket key, best first.
    pub candidates: Vec<String>,
    /// Every branch that exists, to validate a manually linked branch.
    pub known_branches: Vec<String>,
    pub baselines: Baselines,
    pub overlay: Option<ComposerOverlayConfig>,
    /// `[activate].after` task names.
    pub after: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoAction {
    /// The repo has the ticket's branch: switch to it.
    SwitchToTicketBranch(String),
    /// The repo does not touch the ticket: go to this baseline and bring it up to date.
    FallBackToBaseline(String),
}

impl RepoAction {
    pub fn branch(&self) -> &str {
        match self {
            RepoAction::SwitchToTicketBranch(b) | RepoAction::FallBackToBaseline(b) => b,
        }
    }

    pub fn role(&self) -> RepoRole {
        match self {
            RepoAction::SwitchToTicketBranch(_) => RepoRole::Ticket,
            RepoAction::FallBackToBaseline(_) => RepoRole::Baseline,
        }
    }
}

/// The overlay to apply in a consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayPlan {
    /// Only packages whose provider is on the ticket branch.
    pub packages: Vec<OverlayPackage>,
    /// Task names to run once the overlay is in place.
    pub rebuild: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPlan {
    pub name: String,
    pub dir: PathBuf,
    pub action: RepoAction,
    pub overlay: Option<OverlayPlan>,
    /// `[activate].after` tasks; only planned for repos on a ticket branch, and without the
    /// ones the overlay already rebuilds.
    pub after: Vec<String>,
}

impl RepoPlan {
    /// Every task this repo runs, in order: overlay rebuilds, then `after`.
    pub fn tasks(&self) -> impl Iterator<Item = &str> {
        self.overlay
            .iter()
            .flat_map(|o| o.rebuild.iter())
            .chain(self.after.iter())
            .map(String::as_str)
    }
}

/// What activating will do, in the order it will do it: providers before their consumers,
/// otherwise the order the repos were given in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationPlan {
    pub ticket: TicketKey,
    pub baseline: BaselineChoice,
    pub repos: Vec<RepoPlan>,
    /// Things that do not stop the activation but that the user should hear about.
    pub warnings: Vec<String>,
}

/// One reason the plan cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanProblem {
    /// Several branches match and no manual link picks one.
    AmbiguousBranches {
        repo: String,
        candidates: Vec<String>,
    },
    /// The repo needs a baseline branch and has none.
    NoBaseline {
        repo: String,
        choice: BaselineChoice,
    },
    /// A manual link names a branch that does not exist.
    LinkedBranchMissing { repo: String, branch: String },
    /// A manual link names a repo that is not in the workspace.
    LinkedRepoUnknown { repo: String },
    /// Providers and consumers depend on each other in a circle.
    OverlayCycle { repos: Vec<String> },
}

impl std::fmt::Display for PlanProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanProblem::AmbiguousBranches { repo, candidates } => write!(
                f,
                "{repo} has several branches for this ticket ({}); pick one with \
                 `de ticket link <KEY> {repo} --branch <BRANCH>` or hide the repo with --exclude",
                candidates.join(", ")
            ),
            PlanProblem::NoBaseline { repo, choice } => write!(
                f,
                "{repo} has no {choice} branch to fall back to; set it under [branches] in its de.toml"
            ),
            PlanProblem::LinkedBranchMissing { repo, branch } => {
                write!(
                    f,
                    "{repo} is linked to branch '{branch}', which does not exist"
                )
            }
            PlanProblem::LinkedRepoUnknown { repo } => {
                write!(
                    f,
                    "the ticket is linked to '{repo}', which is not in this workspace"
                )
            }
            PlanProblem::OverlayCycle { repos } => write!(
                f,
                "the overlay configuration of {} depends on itself in a circle",
                repos.join(", ")
            ),
        }
    }
}

/// Every problem found, so the user can fix them all at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanError {
    pub problems: Vec<PlanProblem>,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot plan the activation:")?;
        for problem in &self.problems {
            write!(f, "\n  - {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PlanError {}

/// Plans an activation.
///
/// Per repo, first match wins: an `Excluded` link means the ticket does not touch it; a
/// `Manual` link with a branch names the branch (overriding discovery); otherwise the
/// matching branches decide: none means baseline, one means that branch, several is
/// an [`PlanProblem::AmbiguousBranches`]. `Auto` links carry nothing the fresh `candidates`
/// do not already say.
///
/// `baseline` picks the fallback for untouched repos. Normal tickets pass `Base`; for a
/// hotfix the caller asks the user each time.
pub fn plan_activation(
    ticket: &TicketKey,
    repos: &[PlanRepo],
    links: &[RepoLink],
    baseline: BaselineChoice,
) -> Result<ActivationPlan, PlanError> {
    let mut problems = Vec::new();
    let mut warnings = Vec::new();

    for link in links {
        if link.origin == RepoLinkOrigin::Manual && !repos.iter().any(|r| r.name == link.repo) {
            problems.push(PlanProblem::LinkedRepoUnknown {
                repo: link.repo.clone(),
            });
        }
    }

    // 1. What each repo does.
    let mut actions: Vec<Option<RepoAction>> = Vec::with_capacity(repos.len());
    for repo in repos {
        let link = links.iter().find(|l| l.repo == repo.name);
        let ticket_branch = match link {
            Some(l) if l.origin == RepoLinkOrigin::Excluded => None,
            Some(RepoLink {
                origin: RepoLinkOrigin::Manual,
                branch: Some(branch),
                ..
            }) => {
                if repo.known_branches.contains(branch) {
                    Some(branch.clone())
                } else {
                    problems.push(PlanProblem::LinkedBranchMissing {
                        repo: repo.name.clone(),
                        branch: branch.clone(),
                    });
                    None
                }
            }
            _ => match repo.candidates.as_slice() {
                [] => {
                    if link.is_some_and(|l| l.origin == RepoLinkOrigin::Manual) {
                        warnings.push(format!(
                            "{} is linked to the ticket but has no branch for it; using the baseline",
                            repo.name
                        ));
                    }
                    None
                }
                [only] => Some(only.clone()),
                many => {
                    problems.push(PlanProblem::AmbiguousBranches {
                        repo: repo.name.clone(),
                        candidates: many.to_vec(),
                    });
                    None
                }
            },
        };

        actions.push(match ticket_branch {
            Some(branch) => Some(RepoAction::SwitchToTicketBranch(branch)),
            None => match repo.baselines.get(baseline) {
                Some(branch) => Some(RepoAction::FallBackToBaseline(branch.into())),
                None => {
                    // An ambiguity or bad link was already reported for this repo.
                    let reported = problems.iter().any(|p| match p {
                        PlanProblem::AmbiguousBranches { repo: r, .. }
                        | PlanProblem::LinkedBranchMissing { repo: r, .. } => *r == repo.name,
                        _ => false,
                    });
                    if !reported {
                        problems.push(PlanProblem::NoBaseline {
                            repo: repo.name.clone(),
                            choice: baseline,
                        });
                    }
                    None
                }
            },
        });
    }

    // 2. Overlays: a consumer gets the packages whose provider is on the ticket branch.
    let mut overlays: Vec<Option<OverlayPlan>> = Vec::with_capacity(repos.len());
    let mut provider_indexes: Vec<Vec<usize>> = Vec::with_capacity(repos.len());
    for (index, repo) in repos.iter().enumerate() {
        let mut packages = Vec::new();
        let mut providers = Vec::new();

        if let Some(config) = &repo.overlay {
            for (package, provider) in &config.packages {
                let Some(provider_index) = repos.iter().position(|r| &r.name == provider) else {
                    warnings.push(format!(
                        "{} maps {package} to '{provider}', which is not in this workspace; \
                         no overlay for it",
                        repo.name
                    ));
                    continue;
                };
                if provider_index == index {
                    warnings.push(format!(
                        "{} maps {package} to itself; no overlay for it",
                        repo.name
                    ));
                    continue;
                }
                // Provider on its baseline: the consumer keeps the released package.
                if let Some(RepoAction::SwitchToTicketBranch(_)) = &actions[provider_index] {
                    packages.push(OverlayPackage {
                        package: package.clone(),
                        provider: provider.clone(),
                        provider_dir: repos[provider_index].dir.clone(),
                    });
                    if !providers.contains(&provider_index) {
                        providers.push(provider_index);
                    }
                }
            }
        }

        overlays.push((!packages.is_empty()).then(|| {
            OverlayPlan {
                packages,
                rebuild: repo
                    .overlay
                    .as_ref()
                    .map(|c| dedup(&c.rebuild))
                    .unwrap_or_default(),
            }
        }));
        provider_indexes.push(providers);
    }

    // 3. Providers before consumers, otherwise in the order given.
    let mut order: Vec<usize> = Vec::with_capacity(repos.len());
    let mut remaining: Vec<usize> = (0..repos.len()).collect();
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .position(|&i| provider_indexes[i].iter().all(|p| order.contains(p)));
        match next {
            Some(position) => order.push(remaining.remove(position)),
            None => {
                problems.push(PlanProblem::OverlayCycle {
                    repos: remaining.iter().map(|&i| repos[i].name.clone()).collect(),
                });
                break;
            }
        }
    }

    if !problems.is_empty() {
        return Err(PlanError { problems });
    }

    let plans = order
        .into_iter()
        .map(|i| {
            let action = actions[i]
                .clone()
                .expect("an action exists without problems");
            let overlay = overlays[i].clone();
            let after = if matches!(action, RepoAction::SwitchToTicketBranch(_)) {
                let already: &[String] = overlay.as_ref().map_or(&[], |o| &o.rebuild);
                dedup(&repos[i].after)
                    .into_iter()
                    .filter(|t| !already.contains(t))
                    .collect()
            } else {
                Vec::new()
            };
            RepoPlan {
                name: repos[i].name.clone(),
                dir: repos[i].dir.clone(),
                action,
                overlay,
                after,
            }
        })
        .collect();

    Ok(ActivationPlan {
        ticket: ticket.clone(),
        baseline,
        repos: plans,
        warnings,
    })
}

/// `items` without repeats, keeping the first occurrence of each.
fn dedup(items: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for item in items {
        if !seen.contains(item) {
            seen.push(item.clone());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> TicketKey {
        "PROJ-1".parse().unwrap()
    }

    fn baselines(base: &str) -> Baselines {
        Baselines {
            base: Some(base.into()),
            production: Some("master".into()),
            uat: Some("uat".into()),
        }
    }

    fn repo(name: &str) -> PlanRepo {
        PlanRepo {
            name: name.into(),
            dir: format!("/ws/{name}").into(),
            candidates: vec![],
            known_branches: vec!["develop".into(), "master".into()],
            baselines: baselines("develop"),
            overlay: None,
            after: vec![],
        }
    }

    fn with_branch(mut r: PlanRepo, branch: &str) -> PlanRepo {
        r.candidates.push(branch.into());
        r.known_branches.push(branch.into());
        r
    }

    fn consumer(name: &str, package: &str, provider: &str, rebuild: &[&str]) -> PlanRepo {
        let mut r = repo(name);
        r.overlay = Some(ComposerOverlayConfig {
            packages: [(package.to_string(), provider.to_string())].into(),
            rebuild: rebuild.iter().map(|s| String::from(*s)).collect(),
        });
        r
    }

    fn link(repo: &str, branch: Option<&str>, origin: RepoLinkOrigin) -> RepoLink {
        RepoLink {
            ticket: key(),
            repo: repo.into(),
            branch: branch.map(String::from),
            origin,
        }
    }

    fn plan(repos: &[PlanRepo], links: &[RepoLink]) -> ActivationPlan {
        plan_activation(&key(), repos, links, BaselineChoice::Base).expect("plan")
    }

    fn action<'a>(plan: &'a ActivationPlan, repo: &str) -> &'a RepoAction {
        &plan.repos.iter().find(|r| r.name == repo).unwrap().action
    }

    fn names(plan: &ActivationPlan) -> Vec<&str> {
        plan.repos.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn touched_repos_switch_and_the_rest_fall_back() {
        let repos = [
            with_branch(repo("api"), "feature/PROJ-1-x"),
            repo("web"),
            repo("docs"),
        ];
        let p = plan(&repos, &[]);
        assert_eq!(
            action(&p, "api"),
            &RepoAction::SwitchToTicketBranch("feature/PROJ-1-x".into())
        );
        assert_eq!(
            action(&p, "web"),
            &RepoAction::FallBackToBaseline("develop".into())
        );
        assert_eq!(
            action(&p, "docs"),
            &RepoAction::FallBackToBaseline("develop".into())
        );
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn baseline_uses_each_repos_own_base() {
        let mut worker = repo("worker");
        worker.baselines = baselines("main");
        let p = plan(&[repo("web"), worker], &[]);
        assert_eq!(action(&p, "web").branch(), "develop");
        assert_eq!(action(&p, "worker").branch(), "main");
    }

    #[test]
    fn hotfix_baseline_choices() {
        let mut worker = repo("worker");
        worker.baselines = Baselines {
            base: Some("main".into()),
            production: Some("main".into()),
            uat: Some("integration".into()),
        };
        let repos = [repo("web"), worker];

        for (choice, web, worker) in [
            (BaselineChoice::Base, "develop", "main"),
            (BaselineChoice::Production, "master", "main"),
            (BaselineChoice::Uat, "uat", "integration"),
        ] {
            let p = plan_activation(&key(), &repos, &[], choice).unwrap();
            assert_eq!(p.baseline, choice);
            assert_eq!(action(&p, "web").branch(), web, "{choice}");
            assert_eq!(action(&p, "worker").branch(), worker, "{choice}");
        }
    }

    #[test]
    fn a_repo_without_the_chosen_baseline_is_a_problem() {
        let mut web = repo("web");
        web.baselines.uat = None;
        let err = plan_activation(&key(), &[web], &[], BaselineChoice::Uat).unwrap_err();
        assert_eq!(
            err.problems,
            [PlanProblem::NoBaseline {
                repo: "web".into(),
                choice: BaselineChoice::Uat
            }]
        );
        assert!(err.to_string().contains("web has no uat branch"));

        // A repo on the ticket branch does not need one.
        let mut api = with_branch(repo("api"), "PROJ-1");
        api.baselines = Baselines::default();
        plan(&[api], &[]);
    }

    #[test]
    fn several_matching_branches_are_rejected_listing_the_candidates() {
        let mut api = with_branch(repo("api"), "feature/PROJ-1-old");
        api = with_branch(api, "feature/PROJ-1-new");
        let err =
            plan_activation(&key(), &[api, repo("web")], &[], BaselineChoice::Base).unwrap_err();
        assert_eq!(
            err.problems,
            [PlanProblem::AmbiguousBranches {
                repo: "api".into(),
                candidates: vec!["feature/PROJ-1-old".into(), "feature/PROJ-1-new".into()],
            }]
        );
        let text = err.to_string();
        assert!(
            text.contains("feature/PROJ-1-old, feature/PROJ-1-new"),
            "{text}"
        );
        assert!(text.contains("de ticket link"), "{text}");
    }

    #[test]
    fn a_manual_link_resolves_an_ambiguity_and_overrides_discovery() {
        let mut api = with_branch(repo("api"), "feature/PROJ-1-old");
        api = with_branch(api, "feature/PROJ-1-new");
        let p = plan(
            &[api],
            &[link(
                "api",
                Some("feature/PROJ-1-new"),
                RepoLinkOrigin::Manual,
            )],
        );
        assert_eq!(action(&p, "api").branch(), "feature/PROJ-1-new");

        // Override even when discovery found a different single branch or none at all.
        let mut web = with_branch(repo("web"), "feature/PROJ-1-auto");
        web.known_branches.push("hand-picked".into());
        let mut docs = repo("docs");
        docs.known_branches.push("docs-work".into());
        let p = plan(
            &[web, docs],
            &[
                link("web", Some("hand-picked"), RepoLinkOrigin::Manual),
                link("docs", Some("docs-work"), RepoLinkOrigin::Manual),
            ],
        );
        assert_eq!(action(&p, "web").branch(), "hand-picked");
        assert_eq!(
            action(&p, "docs"),
            &RepoAction::SwitchToTicketBranch("docs-work".into())
        );
    }

    #[test]
    fn a_manual_link_to_a_missing_branch_or_repo_is_a_problem() {
        let err = plan_activation(
            &key(),
            &[repo("web")],
            &[
                link("web", Some("nope"), RepoLinkOrigin::Manual),
                link("ghost", None, RepoLinkOrigin::Manual),
            ],
            BaselineChoice::Base,
        )
        .unwrap_err();
        assert_eq!(
            err.problems,
            [
                PlanProblem::LinkedRepoUnknown {
                    repo: "ghost".into()
                },
                PlanProblem::LinkedBranchMissing {
                    repo: "web".into(),
                    branch: "nope".into()
                },
            ]
        );
    }

    #[test]
    fn a_manual_link_without_a_branch_uses_discovery_or_warns() {
        let p = plan(
            &[with_branch(repo("api"), "PROJ-1"), repo("web")],
            &[
                link("api", None, RepoLinkOrigin::Manual),
                link("web", None, RepoLinkOrigin::Manual),
            ],
        );
        assert_eq!(
            action(&p, "api"),
            &RepoAction::SwitchToTicketBranch("PROJ-1".into())
        );
        assert_eq!(action(&p, "web").branch(), "develop");
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("web is linked"));
    }

    #[test]
    fn excluded_repos_are_ignored_even_with_a_matching_branch() {
        let mut api = with_branch(repo("api"), "feature/PROJ-1-old");
        api = with_branch(api, "feature/PROJ-1-new");
        // Excluding also silences an ambiguity.
        let p = plan(&[api], &[link("api", None, RepoLinkOrigin::Excluded)]);
        assert_eq!(
            action(&p, "api"),
            &RepoAction::FallBackToBaseline("develop".into())
        );

        let p = plan(
            &[with_branch(repo("web"), "PROJ-1")],
            &[link("web", None, RepoLinkOrigin::Excluded)],
        );
        assert_eq!(action(&p, "web").branch(), "develop");
    }

    #[test]
    fn auto_links_add_nothing_beyond_fresh_discovery() {
        let p = plan(
            &[with_branch(repo("web"), "PROJ-1")],
            &[link("web", Some("stale-branch"), RepoLinkOrigin::Auto)],
        );
        assert_eq!(action(&p, "web").branch(), "PROJ-1");
    }

    #[test]
    fn overlay_needs_the_provider_on_the_ticket_branch() {
        let repos = [
            consumer("web", "acme/api-client", "api", &["build-ui"]),
            with_branch(repo("api"), "PROJ-1"),
        ];
        let p = plan(&repos, &[]);
        let web = p.repos.iter().find(|r| r.name == "web").unwrap();
        // The consumer is on its baseline, yet gets the overlay.
        assert_eq!(web.action, RepoAction::FallBackToBaseline("develop".into()));
        let overlay = web.overlay.as_ref().expect("overlay planned");
        assert_eq!(
            overlay.packages,
            [OverlayPackage {
                package: "acme/api-client".into(),
                provider: "api".into(),
                provider_dir: "/ws/api".into(),
            }]
        );
        assert_eq!(overlay.rebuild, ["build-ui"]);
        assert!(
            p.repos
                .iter()
                .find(|r| r.name == "api")
                .unwrap()
                .overlay
                .is_none()
        );
    }

    #[test]
    fn provider_on_baseline_means_no_overlay_even_when_configured() {
        let repos = [
            consumer("web", "acme/api-client", "api", &["build-ui"]),
            repo("api"),
        ];
        let p = plan(&repos, &[]);
        let web = p.repos.iter().find(|r| r.name == "web").unwrap();
        assert!(web.overlay.is_none());
        assert_eq!(web.tasks().count(), 0, "no rebuild without an overlay");
    }

    #[test]
    fn a_consumer_that_is_not_configured_is_left_alone() {
        let repos = [repo("web"), with_branch(repo("api"), "PROJ-1")];
        let p = plan(&repos, &[]);
        assert!(p.repos.iter().all(|r| r.overlay.is_none()));
    }

    #[test]
    fn a_ticket_branch_in_the_consumer_too_still_gets_the_overlay() {
        let repos = [
            with_branch(
                consumer("web", "acme/api-client", "api", &["build-ui"]),
                "PROJ-1-web",
            ),
            with_branch(repo("api"), "PROJ-1"),
        ];
        let p = plan(&repos, &[]);
        let web = p.repos.iter().find(|r| r.name == "web").unwrap();
        assert_eq!(
            web.action,
            RepoAction::SwitchToTicketBranch("PROJ-1-web".into())
        );
        assert!(web.overlay.is_some());
    }

    #[test]
    fn only_packages_from_switched_providers_are_overlaid() {
        let mut web = repo("web");
        web.overlay = Some(ComposerOverlayConfig {
            packages: [
                ("acme/api-client".to_string(), "api".to_string()),
                ("acme/ui-kit".to_string(), "ui".to_string()),
            ]
            .into(),
            rebuild: vec!["build-ui".into()],
        });
        let p = plan(&[web, with_branch(repo("api"), "PROJ-1"), repo("ui")], &[]);
        let overlay = p
            .repos
            .iter()
            .find(|r| r.name == "web")
            .unwrap()
            .overlay
            .clone()
            .unwrap();
        assert_eq!(overlay.packages.len(), 1);
        assert_eq!(overlay.packages[0].package, "acme/api-client");
    }

    #[test]
    fn unknown_or_self_providers_only_warn() {
        let repos = [
            consumer("web", "acme/api-client", "missing", &[]),
            consumer("api", "acme/self", "api", &[]),
        ];
        let p = plan(&repos, &[]);
        assert!(p.repos.iter().all(|r| r.overlay.is_none()));
        assert_eq!(p.warnings.len(), 2);
        assert!(p.warnings[0].contains("not in this workspace"));
        assert!(p.warnings[1].contains("to itself"));
    }

    #[test]
    fn providers_come_before_consumers_and_the_rest_keep_their_order() {
        let repos = [
            consumer("web", "acme/api-client", "api", &[]),
            repo("docs"),
            with_branch(repo("api"), "PROJ-1"),
            repo("worker"),
        ];
        let p = plan(&repos, &[]);
        assert_eq!(names(&p), ["docs", "api", "web", "worker"]);
    }

    #[test]
    fn chains_of_providers_are_ordered_transitively() {
        // web consumes api; api consumes core.
        let repos = [
            consumer("web", "acme/api", "api", &[]),
            with_branch(consumer("api", "acme/core", "core", &[]), "PROJ-1-api"),
            with_branch(repo("core"), "PROJ-1-core"),
        ];
        let p = plan(&repos, &[]);
        assert_eq!(names(&p), ["core", "api", "web"]);
    }

    #[test]
    fn a_provider_off_the_ticket_branch_does_not_constrain_the_order() {
        let repos = [consumer("web", "acme/api-client", "api", &[]), repo("api")];
        assert_eq!(names(&plan(&repos, &[])), ["web", "api"]);
    }

    #[test]
    fn circular_overlays_are_rejected() {
        let repos = [
            with_branch(consumer("a", "x/b", "b", &[]), "PROJ-1"),
            with_branch(consumer("b", "x/a", "a", &[]), "PROJ-1"),
        ];
        let err = plan_activation(&key(), &repos, &[], BaselineChoice::Base).unwrap_err();
        assert_eq!(
            err.problems,
            [PlanProblem::OverlayCycle {
                repos: vec!["a".into(), "b".into()]
            }]
        );
    }

    #[test]
    fn tasks_run_overlay_rebuilds_then_after_without_repeats() {
        let mut web = with_branch(
            consumer(
                "web",
                "acme/api-client",
                "api",
                &["composer-update", "build-ui", "build-ui"],
            ),
            "PROJ-1-web",
        );
        web.after = vec!["build-ui".into(), "warm".into(), "warm".into()];
        let p = plan(&[web, with_branch(repo("api"), "PROJ-1")], &[]);
        let web = p.repos.iter().find(|r| r.name == "web").unwrap();
        assert_eq!(
            web.tasks().collect::<Vec<_>>(),
            ["composer-update", "build-ui", "warm"]
        );
    }

    #[test]
    fn activate_after_only_runs_on_a_ticket_branch() {
        let mut on_ticket = with_branch(repo("worker"), "PROJ-1");
        on_ticket.after = vec!["warm".into()];
        let mut on_base = repo("cron");
        on_base.after = vec!["warm".into()];

        let p = plan(&[on_ticket, on_base], &[]);
        assert_eq!(
            p.repos
                .iter()
                .find(|r| r.name == "worker")
                .unwrap()
                .tasks()
                .collect::<Vec<_>>(),
            ["warm"]
        );
        assert_eq!(
            p.repos
                .iter()
                .find(|r| r.name == "cron")
                .unwrap()
                .tasks()
                .count(),
            0
        );
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let mut two = with_branch(repo("a"), "PROJ-1-x");
        two = with_branch(two, "PROJ-1-y");
        let mut none = repo("b");
        none.baselines = Baselines::default();
        let err = plan_activation(&key(), &[two, none], &[], BaselineChoice::Base).unwrap_err();
        assert_eq!(err.problems.len(), 2);
    }
}

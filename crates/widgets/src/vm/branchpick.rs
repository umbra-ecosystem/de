//! The modal that picks a branch for the whole workspace: which branches its projects have, which
//! ones they fall back to, and what switching to the chosen one would do.
//!
//! Pure view-model work. The rule it shows is the rule `de-core`'s `switch` module carries out —
//! a project that has the branch goes to it, one that does not falls back to the first of its
//! fallbacks it actually has, and one that has neither cannot move — so the preview and the
//! switch cannot disagree. Both stores build this the same way from what their projects have.

use std::collections::BTreeMap;

use super::common::{Btn, ago};
use super::ids::{Branch, RepoName};
use super::intent::{Command, Intent};

/// One project as the picker sees it: what it has and where it falls back to.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchRepoVm {
    pub name: RepoName,
    /// Every branch the project has, local or on a remote.
    pub branches: Vec<Branch>,
    /// Tip commit time of each branch in `branches`, unix seconds (the newest tip among its
    /// refs): what the picker orders by. A branch with no entry is of unknown age and sorts last.
    pub tips: BTreeMap<Branch, i64>,
    /// Where it falls back to when it does not have what was asked for, best first.
    pub fallback: Vec<Branch>,
    /// What it is on now; `None` when it could not be read.
    pub current: Option<Branch>,
}

/// One branch to choose, with why it is worth considering.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchItemVm {
    pub branch: Branch,
    /// What it says beside the name: `in 6 of 8` or `fallback for 6 of 8`.
    pub hint: String,
    pub selected: bool,
    pub pick: Intent,
}

/// One group of branches, under a label.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchGroupVm {
    pub heading: String,
    pub items: Vec<BranchItemVm>,
}

/// What picking a branch would do, in the words that go under the list.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchEffectVm {
    /// `6 of 8 projects will switch to develop.`
    pub summary: String,
    /// Quiet lines: `worker → master · no develop here`.
    pub fallbacks: Vec<String>,
    /// What is off, and what to do about it: `foundation has no branches yet.`
    pub stuck: Vec<String>,
}

/// The modal: the branches to pick from and, once one is picked, what picking it would do.
#[derive(Clone, Debug, PartialEq)]
pub struct BranchPickerVm {
    /// The branches have not been read yet (the first check of the workspace is still running).
    pub reading: bool,
    /// The search as typed.
    pub query: String,
    pub groups: Vec<BranchGroupVm>,
    /// How many branches there are before the search narrows them.
    pub total: usize,
    /// The branch picked; `None` until one is.
    pub chosen: Option<Branch>,
    /// What picking it would do; `None` until one is.
    pub effect: Option<BranchEffectVm>,
    /// Confirm: only enabled once a branch is picked.
    pub switch: Btn,
}

/// What moving this project to `target` does — the same rule `de-core`'s `switch` carries out.
enum Move {
    To,
    Fallback(Branch),
    Nowhere(String),
}

fn move_to(repo: &BranchRepoVm, target: &Branch) -> Move {
    if repo.branches.iter().any(|b| b == target) {
        Move::To
    } else if let Some(branch) = repo.fallback.iter().find(|c| {
        repo.branches
            .iter()
            .any(|have| have.as_str() == c.as_str())
    }) {
        Move::Fallback(branch.clone())
    } else if repo.branches.is_empty() {
        Move::Nowhere(format!("{} has no branches yet.", repo.name))
    } else {
        Move::Nowhere(format!(
            "{} has no {target} branch and none of its fallbacks; set one under [branches] in \
             its de.toml.",
            repo.name
        ))
    }
}

/// The branches of the projects: the ones projects fall back to in a group of their own, and
/// within each group the most recently updated first, so the branch worked on yesterday is not
/// buried under branches nobody has touched in months. A branch counts as updated when any
/// project holding it has a fresh tip for it.
fn candidates(repos: &[BranchRepoVm]) -> Vec<(Branch, Vec<RepoName>, Vec<RepoName>, i64)> {
    let mut out: Vec<(Branch, Vec<RepoName>, Vec<RepoName>, i64)> = Vec::new();
    for repo in repos {
        for branch in &repo.branches {
            let tip = repo.tips.get(branch).copied().unwrap_or(0);
            match out.iter_mut().find(|(b, ..)| b == branch) {
                Some((_, here, _, known)) => {
                    here.push(repo.name.clone());
                    *known = (*known).max(tip);
                }
                None => out.push((branch.clone(), vec![repo.name.clone()], Vec::new(), tip)),
            }
        }
        if let Some(fallback) = repo.fallback.iter().find(|c| {
            repo.branches
                .iter()
                .any(|have| have.as_str() == c.as_str())
        }) && let Some((.., fallback_of, _)) = out.iter_mut().find(|(b, ..)| b == fallback)
        {
            fallback_of.push(repo.name.clone());
        }
    }
    out.sort_by(
        |(branch, _, fallback, tip), (other, _, other_fallback, other_tip)| {
            (!fallback.is_empty())
                .cmp(&(!other_fallback.is_empty()))
                .then_with(|| other_tip.cmp(tip))
                .then_with(|| branch.as_str().cmp(other.as_str()))
        },
    );
    out
}

/// Build the modal. `chosen` is what has been picked in it, so it can say what that would do.
/// `now` is the unix time the ages are told against; `None` tells no ages (but the order still
/// follows how recently each branch was updated).
///
/// Pure: the facts were read elsewhere (a worker thread), so this is safe to call while drawing.
pub fn branch_picker(
    query: &str,
    chosen: Option<&Branch>,
    repos: &[BranchRepoVm],
    reading: bool,
    now: Option<i64>,
) -> BranchPickerVm {
    let total = repos.len();
    let q = query.trim().to_lowercase();
    let all = candidates(repos);
    let mut groups: Vec<BranchGroupVm> = vec![
        BranchGroupVm {
            heading: "Fallback branches".to_string(),
            items: Vec::new(),
        },
        BranchGroupVm {
            heading: "Other branches".to_string(),
            items: Vec::new(),
        },
    ];
    for (branch, here, fallback_of, tip) in all {
        if !q.is_empty() && !branch.to_string().to_lowercase().contains(&q) {
            continue;
        }
        // The age tells why the branch sits where it does; without a clock there is nothing
        // honest to say, so nothing is claimed.
        let age = match (now, tip) {
            (Some(now), tip) if tip > 0 => format!(" · {}", ago((now - tip).max(0))),
            _ => String::new(),
        };
        let group = if fallback_of.is_empty() { 1 } else { 0 };
        groups[group].items.push(BranchItemVm {
            selected: chosen.is_some_and(|c| c == &branch),
            hint: if fallback_of.is_empty() {
                format!("in {} of {total}{age}", here.len())
            } else {
                format!("fallback for {} of {total}{age}", fallback_of.len())
            },
            pick: Intent::PickBranch(branch.clone()),
            branch,
        });
    }
    groups.retain(|g| !g.items.is_empty());

    let effect = chosen.map(|target| {
        let mut fallbacks = Vec::new();
        let mut stuck = Vec::new();
        let mut moving = 0;
        for repo in repos {
            match move_to(repo, target) {
                Move::To => moving += 1,
                Move::Fallback(branch) => fallbacks.push(format!(
                    "{} \u{2192} {branch} \u{b7} no {target} here",
                    repo.name
                )),
                Move::Nowhere(why) => stuck.push(why),
            }
        }
        let summary = if total > 0 && moving == total {
            format!(
                "{total} project{} will switch to {target}.",
                if total == 1 { "" } else { "s" }
            )
        } else {
            format!("{moving} of {total} projects will switch to {target}.")
        };
        BranchEffectVm {
            summary,
            fallbacks,
            stuck,
        }
    });

    let switch = match chosen {
        Some(branch) => Btn::new("Switch", Intent::Do(Command::SwitchBranch(branch.clone())))
            .primary(),
        None => Btn::new("Switch", Intent::Noop)
            .primary()
            .disabled(Some("Pick a branch first.".to_string())),
    };

    BranchPickerVm {
        reading,
        query: query.to_string(),
        groups,
        total,
        chosen: chosen.cloned(),
        effect,
        switch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(
        name: &str,
        branches: &[(&str, i64)],
        fallback: &[&str],
        current: Option<&str>,
    ) -> BranchRepoVm {
        BranchRepoVm {
            name: RepoName::new(name),
            branches: branches.iter().map(|(b, _)| Branch::from(*b)).collect(),
            tips: branches
                .iter()
                .map(|(b, t)| (Branch::from(*b), *t))
                .collect(),
            fallback: fallback.iter().map(|b| Branch::from(*b)).collect(),
            current: current.map(Branch::from),
        }
    }

    /// A clock reading noon, so the ages come out in hours and days.
    const NOW: Option<i64> = Some(1_700_000_000 + 12 * 3600);
    const HOUR: i64 = 3600;
    const DAY: i64 = 24 * HOUR;

    fn names(vm: &BranchPickerVm) -> Vec<String> {
        vm.groups
            .iter()
            .flat_map(|g| g.items.iter().map(|i| i.branch.to_string()))
            .collect()
    }

    fn hints(vm: &BranchPickerVm) -> Vec<String> {
        vm.groups
            .iter()
            .flat_map(|g| g.items.iter().map(|i| i.hint.clone()))
            .collect()
    }

    /// The branches projects fall back to come first and say so; within each group the most
    /// recently updated leads, with its age beside it so the order reads as a reason.
    /// Silence would be no help in a picker.
    #[test]
    fn fallback_branches_lead_and_each_group_is_newest_first() {
        let now = NOW.unwrap();
        let repos = vec![
            repo(
                "web",
                &[
                    ("develop", now - 2 * DAY),
                    ("master", now - 10 * DAY),
                    ("wip/x", now - 3 * HOUR),
                ],
                &["develop"],
                Some("wip/x"),
            ),
            repo("api", &[("develop", now - DAY)], &["develop"], Some("develop")),
            repo(
                "docs",
                &[("master", now - 10 * DAY)],
                &["master"],
                Some("master"),
            ),
        ];
        let vm = branch_picker("", None, &repos, false, NOW);
        assert_eq!(
            vm.groups
                .iter()
                .map(|g| g.heading.as_str())
                .collect::<Vec<_>>(),
            vec!["Fallback branches", "Other branches"]
        );
        // `develop` was touched yesterday, `master` a month ago; `wip/x` is no fallback.
        assert_eq!(names(&vm), vec!["develop", "master", "wip/x"]);
        assert_eq!(
            hints(&vm),
            vec![
                "fallback for 2 of 3 · 1d ago".to_string(),
                "fallback for 1 of 3 · 10d ago".to_string(),
                "in 1 of 3 · 3h ago".to_string()
            ]
        );
        assert_eq!(vm.total, 3);
        assert_eq!(vm.chosen, None);
        assert_eq!(vm.effect, None);
        assert!(!vm.switch.enabled, "nothing to switch to yet");
    }

    /// Recency outranks how widely a branch is held: a branch one project touched this morning
    /// leads one every project has but nobody touched in a month.
    #[test]
    fn a_fresh_branch_leads_a_stale_one_every_project_has() {
        let now = NOW.unwrap();
        let repos = vec![
            repo(
                "web",
                &[("develop", now - 10 * DAY), ("wip/x", now - HOUR)],
                &["develop"],
                None,
            ),
            repo("api", &[("develop", now - 10 * DAY)], &["develop"], None),
        ];
        let vm = branch_picker("", None, &repos, false, NOW);
        // `develop` is the only fallback so it still leads its group; `wip/x` leads the other.
        assert_eq!(names(&vm), vec!["develop", "wip/x"]);
        assert_eq!(
            hints(&vm),
            vec![
                "fallback for 2 of 2 · 10d ago".to_string(),
                "in 1 of 2 · 1h ago".to_string()
            ]
        );
    }

    /// Without a clock there are no ages to tell, so none are claimed — but the order still
    /// follows how recently each branch was updated.
    #[test]
    fn without_a_clock_there_are_no_ages_but_the_order_still_holds() {
        let repos = vec![
            repo(
                "web",
                &[("develop", 100), ("wip/x", 300)],
                &["develop"],
                None,
            ),
            repo("api", &[("develop", 200)], &["develop"], None),
        ];
        let vm = branch_picker("", None, &repos, false, None);
        assert_eq!(names(&vm), vec!["develop", "wip/x"]);
        assert_eq!(
            hints(&vm),
            vec![
                "fallback for 2 of 2".to_string(),
                "in 1 of 2".to_string()
            ]
        );
    }

    /// The same rule `de-core` carries out: pick it and the preview says who moves, who falls
    /// back and to where, and who cannot move at all.
    #[test]
    fn picking_a_branch_says_who_moves_who_falls_back_and_who_is_stuck() {
        let now = NOW.unwrap();
        let repos = vec![
            repo(
                "web",
                &[("develop", now - DAY), ("master", now - DAY)],
                &["develop"],
                Some("master"),
            ),
            repo("api", &[("develop", now - DAY)], &["develop"], Some("develop")),
            repo(
                "docs",
                &[("master", now - DAY)],
                &["develop", "master"],
                Some("master"),
            ),
            repo("foundation", &[("other", now - DAY)], &["develop"], None),
        ];
        let vm = branch_picker("", Some(&Branch::from("develop")), &repos, false, NOW);
        let effect = vm.effect.as_ref().expect("a choice was made");
        // `web` and `api` have develop; `docs` does not and falls back to master; `foundation`
        // has neither, so it cannot move and says what to fix.
        assert_eq!(effect.summary, "2 of 4 projects will switch to develop.");
        assert_eq!(effect.fallbacks, vec!["docs → master · no develop here".to_string()]);
        assert_eq!(
            effect.stuck,
            vec![
                "foundation has no develop branch and none of its fallbacks; set one under \
                 [branches] in its de.toml."
                    .to_string()
            ]
        );
        assert!(vm.switch.enabled);

        // Every project having it is one sentence, with nothing about fallbacks to read; a
        // single project is still one project.
        let all = vec![repo("web", &[("master", now - DAY)], &["master"], Some("develop"))];
        let vm = branch_picker("", Some(&Branch::from("master")), &all, false, NOW);
        assert_eq!(
            vm.effect.as_ref().unwrap().summary,
            "1 project will switch to master."
        );
    }

    /// The search narrows the list, and a query that matches nothing says so rather than showing
    /// an empty box.
    #[test]
    fn the_search_narrows_and_says_when_nothing_matches() {
        let now = NOW.unwrap();
        let repos = vec![
            repo(
                "web",
                &[("develop", now - DAY), ("wip/x", now - HOUR)],
                &["develop"],
                None,
            ),
            repo("api", &[("develop", now - DAY)], &["develop"], None),
        ];
        let vm = branch_picker("WIP", None, &repos, false, NOW);
        assert_eq!(names(&vm), vec!["wip/x"]);

        let vm = branch_picker("nothing-here", None, &repos, false, NOW);
        assert!(names(&vm).is_empty());
        assert_eq!(vm.total, 2);

        // Before the first read has landed nothing is claimed, not even "no branches".
        let vm = branch_picker("", None, &[], true, NOW);
        assert!(vm.reading);
        assert!(vm.groups.is_empty());
    }
}

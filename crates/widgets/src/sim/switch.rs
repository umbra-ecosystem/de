//! Moving every project of the open workspace to one branch, and where each one goes when it
//! does not have it.
//!
//! The rule is the one `de-core` carries out for real: a project that has the branch goes to it,
//! one that does not goes to its own baseline, and one that has neither is refused with what to
//! fix. Nothing here touches the outside world — the steps take believable time and report what
//! they found, and the branches a project "has" are the invented ones the tickets and the
//! project's own configuration imply.

use super::Sim;
use super::model::RepoCfg;
use crate::vm::{Branch, BranchRepoVm};

impl Sim {
    /// Every branch this project has: its baseline and its production branch, the one it is on
    /// now, and the branches of the tickets that touch it.
    pub(crate) fn branches_of(&self, r: &RepoCfg) -> Vec<Branch> {
        let mut out: Vec<Branch> = vec![r.base.clone(), r.prod.clone()];
        if let Some(now) = self.ws.branches.get(&r.name) {
            out.push(now.clone());
        }
        for t in &self.tickets {
            if let Some(list) = t.cands.get(&r.name) {
                out.extend(list.iter().cloned());
            }
            if let Some(chosen) = t.link.get(&r.name) {
                out.push(chosen.clone());
            }
        }
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out.dedup();
        out
    }

    /// The open workspace's projects, for the picker.
    pub(crate) fn branch_repos(&self) -> Vec<BranchRepoVm> {
        self.repos
            .iter()
            .map(|r| BranchRepoVm {
                name: r.name.clone(),
                branches: self.branches_of(r),
                fallback: vec![r.base.clone()],
                current: self.ws.branches.get(&r.name).cloned(),
            })
            .collect()
    }

    /// One project's switch step: the line it reports when it goes, where it ends up, and — when
    /// it must not run — the reason, which is the friendly one `de-core` gives too.
    pub(crate) fn switch_here(
        &self,
        r: &RepoCfg,
        target: &Branch,
    ) -> (String, Option<Branch>, Option<String>) {
        let have = self.branches_of(r);
        let current = self.ws.branches.get(&r.name);
        let planned = if have.iter().any(|b| b == target) {
            Some(target.clone())
        } else if have.contains(&r.base) {
            Some(r.base.clone())
        } else {
            None
        };
        let Some(planned) = planned else {
            return (
                String::new(),
                None,
                Some(format!("{} has no branches yet.", r.name)),
            );
        };
        // Already on it is work that needs doing none of, and a dirty tree does not change that.
        if current == Some(&planned) {
            return (format!("already on {planned}"), Some(planned), None);
        }
        if self.ws.dirty.contains(&r.name) {
            return (
                String::new(),
                None,
                Some(format!(
                    "{} has uncommitted changes; commit or stash them first, then switch again.",
                    r.name
                )),
            );
        }
        let was = current
            .map(ToString::to_string)
            .unwrap_or_else(|| "detached".to_string());
        if planned == *target {
            (format!("{was} \u{2192} {planned}"), Some(planned), None)
        } else {
            (
                format!("{was} \u{2192} {planned} \u{b7} no {target} here"),
                Some(planned),
                None,
            )
        }
    }

    /// What moving every project to `target` left behind: the projects on it, the projects that
    /// kept their own branch because the switch was refused there, and one line to say so.
    pub(crate) fn apply_switch(&mut self, target: &Branch) -> (usize, usize, usize) {
        let repos = self.repos.clone();
        let (mut on_target, mut refused, mut total) = (0, 0, 0);
        for r in &repos {
            total += 1;
            let (_, to, fail) = self.switch_here(r, target);
            if fail.is_some() {
                refused += 1;
                continue;
            }
            if let Some(to) = to {
                if &to == target {
                    on_target += 1;
                }
                self.ws.branches.insert(r.name.clone(), to);
            }
        }
        (on_target, refused, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four outcomes of the rule, as the sequence reports them: go, fall back, stay put, and
    /// a project that is not allowed to move because its work would be left behind.
    #[test]
    fn a_project_without_the_branch_falls_back_and_a_dirty_one_is_refused() {
        let mut sim = Sim::new();
        let web = sim.repos.iter().find(|r| r.name == "web").unwrap().clone();
        let target = Branch::from("web/does-not-exist");
        // Somewhere other than its baseline, so a move is a move and not "already there".
        sim.ws.dirty.remove(&web.name);
        sim.ws
            .branches
            .insert(web.name.clone(), web.prod.clone());

        let (detail, to, fail) = sim.switch_here(&web, &target);
        assert_eq!(fail, None, "a clean project switches");
        assert_eq!(to.as_deref(), Some(web.base.as_str()));
        assert!(
            detail.contains("no web/does-not-exist here"),
            "{detail}"
        );

        // With its work in the way, the baseline is not worth stashing for: it stays put.
        sim.ws.dirty.insert(web.name.clone());
        let (_, to, fail) = sim.switch_here(&web, &target);
        assert_eq!(to, None);
        assert_eq!(
            fail,
            Some(
                "web has uncommitted changes; commit or stash them first, then switch again."
                    .to_string()
            )
        );

        // Already on the branch is nothing to do, dirty tree or not.
        sim.ws
            .branches
            .insert(web.name.clone(), web.base.clone());
        let (detail, _, fail) = sim.switch_here(&web, &web.base);
        assert_eq!(fail, None);
        assert_eq!(detail, format!("already on {}", web.base));
    }
}

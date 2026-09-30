//! Pure questions about the simulated world. Nothing here mutates (except `lock_busy`, which audits a refusal).

use super::Sim;
use super::model::*;
use crate::vm::{Baseline, Block, Busy, Group, LineAnchor, RepoName, TicketKey};

#[derive(Clone, Debug)]
pub struct PlanRow {
    pub repo: RepoName,
    pub cands: Vec<crate::vm::Branch>,
    pub excluded: bool,
    pub manual: Option<crate::vm::Branch>,
    pub chosen: Option<crate::vm::Branch>,
    pub ambiguous: bool,
}

#[derive(Clone, Debug)]
pub struct Gap {
    pub none: bool,
    pub repos: Vec<(RepoName, crate::vm::Branch)>,
}

#[derive(Clone, Debug)]
pub struct OpenThread {
    pub repo: RepoName,
    pub id: ThreadId,
    pub file: String,
    pub line: Option<LineAnchor>,
    pub author: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub enum UatState {
    OnUat,
    Conflict { with: TicketKey, files: Vec<String> },
    Overlap(Vec<(TicketKey, String)>),
    Clean,
}

#[derive(Clone, Debug)]
pub struct ActRow {
    pub repo: RepoName,
    pub ticket_role: bool,
    pub branch: crate::vm::Branch,
}

#[derive(Clone, Debug)]
pub struct ActPlan {
    pub errors: Vec<String>,
    pub rows: Vec<ActRow>,
    pub overlay: Option<Overlay>,
}

pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .collect()
}

pub fn block_text(b: &Block) -> String {
    let raw = match b {
        Block::List(items) => items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Para(x) | Block::Heading(x) | Block::Code(x) => x.clone(),
    };
    raw.replace("@[you]", "@you")
}

pub fn mentions_me(c: &Comment) -> bool {
    c.body
        .iter()
        .any(|b| matches!(b, Block::Para(x) if x.to_lowercase().contains("@[you]")))
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

impl Sim {
    pub(crate) fn is_hotfix(&self, t: &Ticket) -> bool {
        t.prs.iter().any(|p| {
            let r = self.repo(&p.repo);
            r.prod != r.base && p.dst == r.prod
        })
    }

    pub(crate) fn max_n(t: &Ticket) -> u32 {
        t.comments.iter().map(|c| c.n).max().unwrap_or(0)
    }

    pub(crate) fn unseen_mentions(t: &Ticket) -> Vec<&Comment> {
        t.comments
            .iter()
            .filter(|c| c.n > t.seen_n && c.who != ME && mentions_me(c))
            .collect()
    }

    pub(crate) fn unseen_count(t: &Ticket) -> usize {
        t.comments
            .iter()
            .filter(|c| c.n > t.seen_n && c.who != ME)
            .count()
    }

    pub(crate) fn duration_ms(&self, t: &Ticket) -> i64 {
        t.time_ms + t.act().map_or(0, |a| self.ms - a.started_ms)
    }

    pub(crate) fn fmt_dur(ms: i64) -> String {
        let m = ms / MS_PER_MIN;
        if m >= 60 {
            format!("{}h {:02}m", m / 60, m % 60)
        } else {
            format!("{m}m")
        }
    }

    /* ---------------------------- groups ---------------------------- */

    pub(crate) fn in_group(t: &Ticket, g: Group) -> bool {
        let returned = t.jira == JiraStatus::Returned;
        match g {
            Group::Pool => t.local().is_none() && t.jira == JiraStatus::InReview,
            Group::Mine => {
                matches!(t.local(), Some(Local::Claimed | Local::Reviewing)) && !returned
            }
            Group::Active => t.local() == Some(Local::Active),
            Group::Parked => t.local() == Some(Local::Parked),
            Group::Awaiting => t.local() == Some(Local::Integrated) && !returned,
            Group::Returned => returned,
            Group::Done => t.local() == Some(Local::Done),
            Group::All => true,
        }
    }

    /* ---------------------------- repo plan ---------------------------- */

    pub(crate) fn repo_plan(&self, t: &Ticket) -> Vec<PlanRow> {
        self.repos
            .iter()
            .map(|r| {
                let cands = t.cands.get(&r.name).cloned().unwrap_or_default();
                let excluded = t.excl.contains(&r.name);
                let manual = t.link.get(&r.name).cloned();
                let chosen = if excluded {
                    None
                } else {
                    manual
                        .clone()
                        .or_else(|| (cands.len() == 1).then(|| cands[0].clone()))
                };
                let ambiguous = !excluded && manual.is_none() && cands.len() > 1;
                PlanRow {
                    repo: r.name.clone(),
                    cands,
                    excluded,
                    manual,
                    chosen,
                    ambiguous,
                }
            })
            .filter(|p| !p.cands.is_empty() || p.excluded)
            .collect()
    }

    pub(crate) fn touched(&self, t: &Ticket) -> Vec<PlanRow> {
        self.repo_plan(t)
            .into_iter()
            .filter(|p| p.chosen.is_some())
            .collect()
    }

    /* ---------------------------- missing PRs ---------------------------- */

    fn pr_stage(t: &Ticket) -> bool {
        t.jira == JiraStatus::InReview
            && t.landings().is_empty()
            && matches!(
                t.local(),
                None | Some(Local::Claimed | Local::Reviewing | Local::Parked)
            )
    }

    pub(crate) fn pr_gap(&self, t: &Ticket) -> Option<Gap> {
        if !Self::pr_stage(t) {
            return None;
        }
        let miss: Vec<_> = self
            .touched(t)
            .into_iter()
            .filter(|p| !t.prs.iter().any(|x| x.repo == p.repo))
            .filter_map(|p| Some((p.repo, p.chosen?)))
            .collect();
        if t.prs.is_empty() {
            return Some(Gap {
                none: true,
                repos: miss,
            });
        }
        (!miss.is_empty()).then_some(Gap {
            none: false,
            repos: miss,
        })
    }

    pub(crate) fn gap_text(&self, t: &Ticket) -> String {
        let Some(g) = self.pr_gap(t) else {
            return String::new();
        };
        if g.none {
            if g.repos.is_empty() {
                "No pull request and no ticket branch found.".to_string()
            } else {
                format!(
                    "No pull request yet. {} but no PR.",
                    g.repos
                        .iter()
                        .map(|(r, b)| format!("{r} has {b}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        } else {
            format!(
                "No pull request yet in {}.",
                g.repos
                    .iter()
                    .map(|(r, b)| format!("{r} ({b})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }

    pub(crate) fn gap_error(&self, t: &Ticket) -> Option<String> {
        self.pr_gap(t)
            .map(|_| format!("{} Review can start once it exists.", self.gap_text(t)))
    }

    pub(crate) fn wait_left(&self, w: Option<Wait>) -> Option<i64> {
        w.map(|w| w.since + i64::from(w.mins) * MS_PER_MIN - self.ms)
    }

    pub(crate) fn pr_wait_expired(&self, t: &Ticket) -> bool {
        self.pr_gap(t).is_some() && self.wait_left(t.pr_wait).is_some_and(|l| l <= 0)
    }

    pub(crate) fn return_body(&self, t: &Ticket) -> String {
        let g = self.pr_gap(t);
        let none = g.as_ref().is_some_and(|g| g.none);
        let mut lines = vec![format!(
            "Returned to development: {}",
            if none {
                "no pull request was found for this ticket."
            } else {
                "some branches have no pull request."
            }
        )];
        match &g {
            Some(g) if !g.repos.is_empty() => lines.push(format!(
                "Branches without a PR: {}.",
                g.repos
                    .iter()
                    .map(|(r, b)| format!("{r} ({b})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            _ => lines.push(
                "No ticket branch or pull request was found in the workspace repositories."
                    .to_string(),
            ),
        }
        if let Some(w) = t.pr_wait {
            lines.push(format!(
                "Waited {} min for it to appear (since {}).",
                w.mins,
                self.clock(Some(w.since))
            ));
        }
        lines.push(format!(
            "Please open a pull request for each branch and move the ticket back to {}.",
            JiraStatus::InReview.label()
        ));
        lines.join("\n")
    }

    /* ---------------------------- review threads ---------------------------- */

    fn thread_stage(t: &Ticket) -> bool {
        matches!(
            t.local(),
            Some(Local::Claimed | Local::Reviewing | Local::Parked)
        ) && t.jira == JiraStatus::InReview
            && t.landings().is_empty()
    }

    pub(crate) fn open_threads(t: &Ticket) -> Vec<OpenThread> {
        t.prs
            .iter()
            .flat_map(|p| {
                p.threads
                    .iter()
                    .filter(|th| !th.resolved)
                    .map(|th| OpenThread {
                        repo: p.repo.clone(),
                        id: th.id,
                        file: th.file.clone(),
                        line: th.line,
                        author: th.author.clone(),
                        text: th.text.clone(),
                    })
            })
            .collect()
    }

    pub(crate) fn blocking_threads(t: &Ticket) -> Vec<OpenThread> {
        if !Self::thread_stage(t) {
            return Vec::new();
        }
        Self::open_threads(t)
            .into_iter()
            .filter(|x| !t.accepted.contains(&x.id))
            .collect()
    }

    pub(crate) fn accepted_threads(t: &Ticket) -> Vec<OpenThread> {
        Self::open_threads(t)
            .into_iter()
            .filter(|x| t.accepted.contains(&x.id))
            .collect()
    }

    pub(crate) fn thread_line(x: &OpenThread) -> String {
        let line = x.line.map_or(String::new(), |a| format!(":{}", a.line()));
        format!("{} {}{} ({}): {}", x.repo, x.file, line, x.author, x.text)
    }

    pub(crate) fn th_wait_expired(&self, t: &Ticket) -> bool {
        !Self::blocking_threads(t).is_empty() && self.wait_left(t.th_wait).is_some_and(|l| l <= 0)
    }

    pub(crate) fn return_threads_body(&self, t: &Ticket) -> String {
        let th = Self::blocking_threads(t);
        let many = th.len() > 1;
        let mut lines = vec![format!(
            "Returned to development: {} review comment{} still unresolved.",
            th.len(),
            if many { "s are" } else { " is" }
        )];
        for x in &th {
            lines.push(format!("• {}", Self::thread_line(x)));
        }
        if let Some(w) = t.th_wait {
            lines.push(format!(
                "Waited {} min for {} to be resolved (since {}).",
                w.mins,
                if many { "them" } else { "it" },
                self.clock(Some(w.since))
            ));
        }
        lines.push(format!(
            "Please resolve or answer them, then move the ticket back to {}.",
            JiraStatus::InReview.label()
        ));
        lines.join("\n")
    }

    /* ---------------------------- uat ---------------------------- */

    pub(crate) fn overlaps_for(&self, t: &Ticket, repo: &RepoName) -> Vec<(TicketKey, String)> {
        let mine: Vec<&FileDiff> = t
            .prs
            .iter()
            .filter(|x| x.repo == *repo)
            .flat_map(|x| x.files.iter())
            .collect();
        let mut out = Vec::new();
        for o in &self.tickets {
            if o.key == t.key || !o.has_landed(repo) {
                continue;
            }
            for f in o
                .prs
                .iter()
                .filter(|x| x.repo == *repo)
                .flat_map(|x| x.files.iter())
            {
                if mine.iter().any(|g| g.path == f.path) {
                    out.push((o.key.clone(), f.path.clone()));
                }
            }
        }
        out
    }

    pub(crate) fn uat_check(&self, t: &Ticket) -> Vec<(RepoName, UatState)> {
        self.touched(t)
            .into_iter()
            .map(|p| {
                let repo = p.repo;
                let state = if t.has_landed(&repo) {
                    UatState::OnUat
                } else if self.sims.conflict.as_ref() == Some(&repo) {
                    UatState::Conflict {
                        with: "PROJ-127".into(),
                        files: vec!["src/session.rs".to_string()],
                    }
                } else {
                    let ov = self.overlaps_for(t, &repo);
                    if ov.is_empty() {
                        UatState::Clean
                    } else {
                        UatState::Overlap(ov)
                    }
                };
                (repo, state)
            })
            .collect()
    }

    pub(crate) fn pre_merge_stage(t: &Ticket) -> bool {
        matches!(
            t.local(),
            Some(Local::Claimed | Local::Reviewing | Local::Parked | Local::Active)
        )
    }

    pub(crate) fn uat_conflicts(&self, t: &Ticket) -> Vec<(RepoName, TicketKey, Vec<String>)> {
        if !Self::pre_merge_stage(t) {
            return Vec::new();
        }
        self.uat_check(t)
            .into_iter()
            .filter_map(|(repo, s)| match s {
                UatState::Conflict { with, files } => Some((repo, with, files)),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn uat_overlaps(&self, t: &Ticket) -> bool {
        Self::pre_merge_stage(t)
            && self
                .uat_check(t)
                .iter()
                .any(|(_, s)| matches!(s, UatState::Overlap(_)))
    }

    pub(crate) fn conflict_sig(&self, t: &Ticket) -> String {
        self.uat_conflicts(t)
            .iter()
            .map(|(r, w, _)| format!("{r}:{w}"))
            .collect::<Vec<_>>()
            .join(",")
    }

    pub(crate) fn conflict_sent(&self, t: &Ticket) -> Option<String> {
        t.conflict_sent
            .as_ref()
            .filter(|(sig, _)| *sig == self.conflict_sig(t))
            .map(|(_, at)| at.clone())
    }

    pub(crate) fn conflict_body(&self, t: &Ticket) -> String {
        let cs = self.uat_conflicts(t);
        let mut lines = vec![format!(
            "{} does not merge into uat: it conflicts with another ticket that is already there.",
            t.key
        )];
        for (repo, with, files) in &cs {
            lines.push(format!(
                "• {repo}: conflicts with {with} in {}",
                files.join(", ")
            ));
        }
        let mut owners: Vec<&str> = cs.iter().map(|(_, w, _)| w.as_str()).collect();
        owners.dedup();
        lines.push(format!(
            "Please rebase the ticket branch on uat, or sort it out with the owner of {}.",
            owners.join(", ")
        ));
        lines.join("\n")
    }

    /* ---------------------------- activation plan ---------------------------- */

    pub(crate) fn baseline_for(
        &self,
        repo: &RepoName,
        hotfix: bool,
        choice: Option<Baseline>,
    ) -> crate::vm::Branch {
        let r = self.repo(repo);
        if !hotfix {
            return r.base.clone();
        }
        match choice {
            Some(Baseline::Production) => r.prod.clone(),
            Some(Baseline::Uat) => "uat".into(),
            _ => r.base.clone(),
        }
    }

    pub(crate) fn activation_plan(&self, t: &Ticket, choice: Option<Baseline>) -> ActPlan {
        let mut errors = Vec::new();
        let mut rows = Vec::new();
        if let Some(e) = self.gap_error(t) {
            errors.push(e);
        }
        let plans = self.repo_plan(t);
        let hot = self.is_hotfix(t);
        for r in &self.repos {
            let p = plans.iter().find(|p| p.repo == r.name);
            if let Some(p) = p {
                if p.ambiguous {
                    errors.push(format!(
                        "{}: {} branches match {}. Choose one on the Overview tab.",
                        r.name,
                        p.cands.len(),
                        t.key
                    ));
                }
                if let Some(b) = &p.chosen {
                    rows.push(ActRow {
                        repo: r.name.clone(),
                        ticket_role: true,
                        branch: b.clone(),
                    });
                    continue;
                }
            }
            rows.push(ActRow {
                repo: r.name.clone(),
                ticket_role: false,
                branch: self.baseline_for(&r.name, hot, choice),
            });
        }
        let overlay = rows.iter().find_map(|r| {
            let cfg = self.repo(&r.repo);
            let provider = cfg.consumes.clone()?;
            (cfg.overlay_consumer && rows.iter().any(|x| x.repo == provider && x.ticket_role)).then(
                || Overlay {
                    repo: r.repo.clone(),
                    provider,
                },
            )
        });
        let bt = Self::blocking_threads(t).len();
        if bt > 0 {
            errors.push(format!(
                "{bt} unresolved review comment{}. Wait for them to be resolved, return the ticket, or proceed anyway from the ticket page.",
                plural(bt)
            ));
        }
        ActPlan {
            errors,
            rows,
            overlay,
        }
    }

    /* ---------------------------- pre-push and integration ---------------------------- */

    /// `(repo, index, text, ticked)` for every check of every touched repo.
    pub(crate) fn prepush_items(&self, t: &Ticket) -> Vec<(RepoName, usize, String, bool)> {
        self.touched(t)
            .into_iter()
            .flat_map(|p| {
                let cfg = self.repo(&p.repo).clone();
                cfg.checks
                    .iter()
                    .enumerate()
                    .map(|(i, text)| {
                        (
                            p.repo.clone(),
                            i,
                            (*text).to_string(),
                            t.pre.contains(&(p.repo.clone(), i)),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub(crate) fn prepush_missing(&self, t: &Ticket) -> Vec<(RepoName, String)> {
        self.prepush_items(t)
            .into_iter()
            .filter(|x| !x.3)
            .map(|x| (x.0, x.2))
            .collect()
    }

    pub(crate) fn prep_ready(t: &Ticket) -> bool {
        t.prep().is_some_and(|p| {
            p.rows
                .iter()
                .any(|r| matches!(r.outcome, PrepOutcome::Ready { .. }))
                && p.rows.iter().all(|r| {
                    matches!(
                        r.outcome,
                        PrepOutcome::Ready { .. } | PrepOutcome::AlreadyPushed { .. }
                    )
                })
        })
    }

    pub(crate) fn prep_pushable(&self, t: &Ticket) -> bool {
        Self::prep_ready(t) && self.prepush_missing(t).is_empty()
    }

    pub(crate) fn pushed_all(&self, t: &Ticket) -> bool {
        let touched = self.touched(t);
        !touched.is_empty() && touched.iter().all(|p| t.has_landed(&p.repo))
    }

    pub(crate) fn all_deployed(&self, t: &Ticket) -> bool {
        !t.landings().is_empty()
            && self.pushed_all(t)
            && t.landings()
                .iter()
                .all(|l| l.deploy.state == DeployState::Deployed)
    }

    pub(crate) fn failed_repos(t: &Ticket) -> Vec<RepoName> {
        t.landings()
            .iter()
            .filter(|l| l.deploy.state == DeployState::Failed)
            .map(|l| l.repo.clone())
            .collect()
    }

    pub(crate) fn any_running(t: &Ticket) -> bool {
        t.landings()
            .iter()
            .any(|l| matches!(l.deploy.state, DeployState::Pending | DeployState::Running))
    }

    pub(crate) fn draft(t: &Ticket) -> Option<&Draft> {
        t.drafts().last()
    }

    pub(crate) fn stale_review(t: &Ticket) -> bool {
        t.review_mark()
            .is_some_and(|m| t.prs.iter().any(|p| p.updated_seq > m.0))
    }

    /// The highest PR update sequence: what a fresh review mark covers.
    pub(crate) fn latest_seq(t: &Ticket) -> u32 {
        t.prs.iter().map(|p| p.updated_seq).max().unwrap_or(0)
    }

    pub(crate) fn lock_text(l: &Lock) -> String {
        match l.kind {
            LockKind::Sync => "background sync is fetching".to_string(),
            LockKind::External => format!("{} is running {}", l.by, l.op),
            LockKind::Stale => "a leftover .git/index.lock".to_string(),
        }
    }

    /// A repo is busy: audit the refusal and describe it. Reads take no lock.
    pub(crate) fn lock_busy(
        &mut self,
        repos: &[RepoName],
        op: &str,
        ticket: &TicketKey,
    ) -> Option<Busy> {
        let mut names: Vec<RepoName> = repos.to_vec();
        names.sort();
        names.dedup();
        for r in names {
            let Some(l) = self.locks.get(&r).cloned() else {
                continue;
            };
            self.audit(
                "lock.denied",
                Some(ticket),
                Some(&r),
                AuditOutcome::Failure,
                &format!("{op} refused: {}", Self::lock_text(&l)),
            );
            let stale = matches!(l.kind, LockKind::Stale);
            return Some(Busy {
                message: if stale {
                    format!(
                        "{r} has a leftover .git/index.lock and no git process is running. Remove it to continue. Nothing was changed."
                    )
                } else {
                    format!("{r} is busy: {}. Nothing was changed.", Self::lock_text(&l))
                },
                repo: r,
                stale,
            });
        }
        None
    }
}

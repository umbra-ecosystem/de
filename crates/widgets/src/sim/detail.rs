//! View models for the ticket screen: header, Overview, Review, Test, Ship.

use super::Sim;
use super::model::*;
use super::queries::{UatState, initials};
use crate::store::ReviewSel;
use crate::vm::*;

pub fn priority_tone(p: Priority) -> Tone {
    match p {
        Priority::Highest => Tone::Bad,
        Priority::High => Tone::Warn,
        _ => Tone::Neutral,
    }
}

pub fn jira_tone(s: JiraStatus) -> Tone {
    match s {
        JiraStatus::Returned => Tone::Warn,
        JiraStatus::AlphaTesting => Tone::Accent,
        JiraStatus::Done => Tone::Ok,
        JiraStatus::InReview | JiraStatus::Other => Tone::Neutral,
    }
}

pub fn local_tone(l: Local) -> Tone {
    match l {
        Local::Active => Tone::Accent,
        Local::Integrated | Local::Done => Tone::Ok,
        Local::Reviewing => Tone::Warn,
        _ => Tone::Neutral,
    }
}

pub fn jira_badge(s: JiraStatus) -> Badge {
    Badge::new(s.label(), jira_tone(s))
}

/// The status badge of a ticket: Jira's own word when the prototype has none.
pub fn jira_badge_of(t: &Ticket) -> Badge {
    match &t.jira_name {
        Some(name) => Badge::new(name.clone(), jira_tone(t.jira)),
        None => jira_badge(t.jira),
    }
}

pub fn local_badge(l: Option<Local>) -> Option<Badge> {
    l.map(|l| Badge::new(l.label(), local_tone(l)))
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

const STEPS: [&str; 9] = [
    "Claimed",
    "Reviewed",
    "Testing",
    "On uat",
    "Deployed",
    "Announced",
    "Alpha",
    "Signed off",
    "Approved",
];

/// A hunk: a run of changes with at most one context line between them.
pub fn hunks_of(f: &FileDiff) -> Vec<(usize, usize)> {
    let ch: Vec<usize> = f
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.k != Kind::Ctx)
        .map(|(i, _)| i)
        .collect();
    if ch.is_empty() {
        return vec![(0, f.lines.len())];
    }
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for c in ch {
        match groups.last_mut() {
            Some(g) if c - g.1 <= 2 => g.1 = c,
            _ => groups.push((c, c)),
        }
    }
    let mut cuts = vec![0usize];
    for i in 1..groups.len() {
        let gap = groups[i].0 - groups[i - 1].1 - 1;
        cuts.push(groups[i - 1].1 + 1 + gap / 2);
    }
    cuts.push(f.lines.len());
    (0..groups.len()).map(|i| (cuts[i], cuts[i + 1])).collect()
}

/// Pairs deleted and added runs for the split view.
fn split_rows(lines: &[Line]) -> Vec<(Option<&Line>, Option<&Line>)> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].k == Kind::Ctx {
            rows.push((Some(&lines[i]), Some(&lines[i])));
            i += 1;
            continue;
        }
        let mut dels = Vec::new();
        let mut adds = Vec::new();
        while i < lines.len() && lines[i].k == Kind::Del {
            dels.push(&lines[i]);
            i += 1;
        }
        while i < lines.len() && lines[i].k == Kind::Add {
            adds.push(&lines[i]);
            i += 1;
        }
        for j in 0..dels.len().max(adds.len()) {
            rows.push((dels.get(j).copied(), adds.get(j).copied()));
        }
    }
    rows
}

fn anchor_of(l: &Line) -> LineAnchor {
    match (l.n, l.o) {
        (Some(n), _) => LineAnchor::New(n),
        (None, Some(o)) => LineAnchor::Old(o),
        (None, None) => LineAnchor::New(0),
    }
}

fn line_vm(l: &Line) -> DiffLineVm {
    DiffLineVm {
        kind: match l.k {
            Kind::Ctx => LineKind::Context,
            Kind::Add => LineKind::Add,
            Kind::Del => LineKind::Del,
        },
        old: l.o,
        new: l.n,
        text: l.x.clone(),
        anchor: anchor_of(l),
    }
}

fn thread_vm(th: &Thread) -> ThreadVm {
    ThreadVm {
        author: th.author.clone(),
        initials: initials(&th.author),
        mine: th.author == ME,
        resolved: th.resolved,
        text: th.text.clone(),
    }
}

/// Synthetic outside URLs for the simulation (example hosts only, never real data).
pub fn jira_url(key: &TicketKey) -> String {
    format!("https://example.atlassian.net/browse/{key}")
}

pub fn pr_url(host: &str, id: PrNumber) -> String {
    format!("https://github.com/{host}/pull/{id}")
}

pub fn run_url(host: &str, run: u32) -> String {
    format!("https://github.com/{host}/actions/runs/{run}")
}

pub fn deploy_chip_for(host: &str, d: &Deploy) -> DeployChip {
    let (text, tone) = match d.state {
        DeployState::Deployed => ("● deployed alpha".to_string(), Tone::Ok),
        DeployState::Failed => ("✕ failed".to_string(), Tone::Bad),
        s => (format!("◐ {}", s.word()), Tone::Warn),
    };
    DeployChip {
        text,
        tone,
        run: d.run,
        step: matches!(d.state, DeployState::Pending | DeployState::Running)
            .then(|| d.step.clone()),
        // The engine's address when it knows one; the synthetic one while it does not.
        url: d
            .url
            .clone()
            .or_else(|| (d.run > 0).then(|| run_url(host, d.run))),
    }
}

/// Why the announce step waits while the ticket is being (re-)integrated.
pub(crate) const REMERGE_FIRST: &str = "Push the re-merge to uat and wait for it to deploy first.";

impl Sim {
    fn progress_index(&self, t: &Ticket) -> f32 {
        let d = Self::draft(t);
        if t.local() == Some(Local::Done) {
            return 9.0;
        }
        if t.local() == Some(Local::Integrated)
            || (!t.landings().is_empty() && t.local() != Some(Local::Active))
        {
            if t.jira.is_signed_off() {
                return 7.0;
            }
            if t.jira == JiraStatus::AlphaTesting {
                return 6.0;
            }
            if d.is_some_and(|d| d.posted) {
                return 5.0;
            }
            if self.all_deployed(t) {
                return 4.0;
            }
            return 3.0;
        }
        if t.local() == Some(Local::Active) {
            return 2.0;
        }
        if t.reviewed() {
            return 1.5;
        }
        if t.local().is_some() {
            return 1.0;
        }
        0.0
    }

    fn stepper(&self, t: &Ticket) -> StepperVm {
        let idx = self.progress_index(t);
        let frac = idx.fract() != 0.0;
        StepperVm {
            steps: STEPS
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let fi = i as f32;
                    let state = if fi < idx {
                        StepState::Done
                    } else if i == idx.floor() as usize || (frac && i == idx.ceil() as usize) {
                        StepState::Now
                    } else {
                        StepState::Todo
                    };
                    ((*s).to_string(), state)
                })
                .collect(),
        }
    }

    fn pr_gap_banner(&self, t: &Ticket) -> Option<BannerVm> {
        self.pr_gap(t)?;
        let left = self.wait_left(t.pr_wait);
        let exp = t.pr_wait.is_some() && left.is_some_and(|l| l <= 0);
        let status = match (t.pr_wait, left) {
            (None, _) => format!("Waiting up to {} minutes for it to appear.", self.wait_min),
            (Some(w), Some(l)) if l <= 0 => {
                format!("Waited {} min and it still has no PR.", w.mins)
            }
            (Some(w), Some(l)) => format!(
                "{} of {} min left. It clears itself when the PR appears.",
                (l + MS_PER_MIN - 1) / MS_PER_MIN,
                w.mins
            ),
            _ => String::new(),
        };
        let mut actions = Vec::new();
        if t.pr_wait.is_some() {
            actions.push(
                Btn::new(
                    "Return with comment…",
                    Intent::Do(Command::ReturnMissingPr(t.key.clone())),
                )
                .primary_if(exp),
            );
            actions.push(Btn::new(
                format!("Wait {} more min", self.wait_min),
                Intent::Do(Command::ExtendWait {
                    key: t.key.clone(),
                    kind: WaitKind::PullRequest,
                }),
            ));
        }
        Some(BannerVm {
            tone: if exp { Tone::Bad } else { Tone::Warn },
            title: format!(
                "{} Missing pull request. Review cannot start.",
                if exp { "✕" } else { "⚠" }
            ),
            lines: vec![format!("{} {status}", self.gap_text(t))],
            actions,
        })
    }

    fn threads_banner(&self, t: &Ticket) -> Option<BannerVm> {
        let bt = Self::blocking_threads(t);
        let acc = Self::accepted_threads(t);
        if bt.is_empty() {
            return (!acc.is_empty()).then(|| BannerVm {
                tone: Tone::Warn,
                title: format!(
                    "⚠ Proceeding with {} unresolved review comment{}.",
                    acc.len(),
                    plural(acc.len())
                ),
                lines: vec!["The deploy comment will list them.".to_string()],
                actions: Vec::new(),
            });
        }
        let exp = self.th_wait_expired(t);
        let left = self.wait_left(t.th_wait);
        let status = match (t.th_wait, left) {
            (None, _) => format!("Waiting up to {} minutes for them to be resolved.", self.wait_min),
            (Some(w), Some(l)) if l <= 0 => {
                format!("Waited {} min and they are still open.", w.mins)
            }
            (Some(w), Some(l)) => format!(
                "{} of {} min left. It clears itself when they are resolved.",
                (l + MS_PER_MIN - 1) / MS_PER_MIN,
                w.mins
            ),
            _ => String::new(),
        };
        let mut lines: Vec<String> = bt.iter().take(3).map(Self::thread_line).collect();
        if bt.len() > 3 {
            lines.push(format!("and {} more", bt.len() - 3));
        }
        lines.push(status);
        let mut actions = Vec::new();
        if t.th_wait.is_some() {
            actions.push(
                Btn::new(
                    "Return with comment…",
                    Intent::Do(Command::ReturnThreads(t.key.clone())),
                )
                .primary_if(exp),
            );
            actions.push(Btn::new(
                format!("Wait {} more min", self.wait_min),
                Intent::Do(Command::ExtendWait {
                    key: t.key.clone(),
                    kind: WaitKind::Threads,
                }),
            ));
        }
        actions.push(Btn::new(
            "Proceed anyway…",
            Intent::Do(Command::AcceptThreads(t.key.clone())),
        ));
        Some(BannerVm {
            tone: if exp { Tone::Bad } else { Tone::Warn },
            title: format!(
                "{} {} unresolved review comment{}. Activation is blocked.",
                if exp { "✕" } else { "⚠" },
                bt.len(),
                plural(bt.len())
            ),
            lines,
            actions,
        })
    }

    pub(crate) fn uat_flag(&self, t: &Ticket) -> Option<Badge> {
        if !self.uat_conflicts(t).is_empty() {
            Some(Badge::new("uat conflict", Tone::Bad))
        } else if self.uat_overlaps(t) {
            Some(Badge::new("overlaps uat", Tone::Warn))
        } else {
            None
        }
    }

    pub(crate) fn vm_head(&self, key: &TicketKey) -> Option<TicketHeadVm> {
        let t = self.tk(key)?;
        let st = t.local();
        let gap = self.pr_gap(t).is_some();
        let gap_hint = gap.then(|| "No pull request yet".to_string());
        let mut actions = Vec::new();
        if st.is_none() {
            actions.push(Btn::new("Claim", Intent::Do(Command::Claim(key.clone()))).primary());
        }
        if st == Some(Local::Claimed) {
            actions.push(
                Btn::new(
                    "Start review",
                    Intent::Do(Command::StartReview(key.clone())),
                )
                .primary()
                .disabled(gap_hint.clone()),
            );
        }
        if matches!(st, Some(Local::Claimed | Local::Reviewing | Local::Parked)) && t.reviewed() {
            actions.push(
                Btn::new(
                    "Activate…",
                    Intent::Do(Command::Activate {
                        key: key.clone(),
                        baseline: None,
                    }),
                )
                .primary()
                .disabled(gap_hint.clone()),
            );
        }
        if st == Some(Local::Reviewing) && !t.reviewed() {
            actions.push(
                Btn::new(
                    "Mark reviewed",
                    Intent::Do(Command::MarkReviewed(key.clone())),
                )
                .primary()
                .disabled(gap_hint),
            );
        }
        if st == Some(Local::Active) {
            actions.push(Btn::new("Park…", Intent::Do(Command::Park(key.clone()))));
        }
        let banners = [self.pr_gap_banner(t), self.threads_banner(t)]
            .into_iter()
            .flatten()
            .collect();
        Some(TicketHeadVm {
            key: key.clone(),
            title: t.title.clone(),
            jira: jira_badge_of(t),
            local: local_badge(st),
            hotfix: self.is_hotfix(t),
            uat_flag: self.uat_flag(t),
            // Only what the engine cached: never a fabricated address. Showcase tickets
            // carry the synthetic one from their seed; unknown stays a static key.
            jira_url: t.jira_url.clone(),
            actions,
            banners,
            stepper: self.stepper(t),
            tabs: TicketTab::LIST
                .iter()
                .map(|tab| (*tab, *tab == TicketTab::Review && Self::stale_review(t)))
                .collect(),
        })
    }

    /* ---------------------------- overview ---------------------------- */

    /// The ticket's attachments that no body references: a file the description or a comment
    /// already shows where it sits stays out of the Attachments band rather than appearing twice.
    fn unreferenced_attachments(t: &Ticket) -> Vec<(String, String)> {
        fn texts(blocks: &[Block]) -> impl Iterator<Item = &str> {
            blocks.iter().flat_map(|b| match b {
                Block::Para(s) | Block::Heading(s) | Block::Quote(s) | Block::Code(s) => {
                    vec![s.as_str()]
                }
                Block::List(items) | Block::Numbered(items) => {
                    items.iter().map(String::as_str).collect()
                }
            })
        }
        let bodies: Vec<&str> = texts(&t.desc)
            .chain(t.comments.iter().flat_map(|c| texts(&c.body)))
            .collect();
        t.attachments
            .iter()
            .filter(|(name, _)| !bodies.iter().any(|b| b.contains(&format!("![{name}]"))))
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    pub(crate) fn vm_overview(&self, key: &TicketKey, seen: Option<u32>) -> Option<OverviewVm> {
        let t = self.tk(key)?;
        let snap = seen.unwrap_or(t.seen_n);
        let plan = self.repo_plan(t);
        let mut repos: Vec<RepoRowVm> = plan
            .iter()
            .map(|p| {
                let pr = t.prs.iter().find(|x| x.repo == p.repo);
                let branch = if p.ambiguous {
                    BranchCell::Ambiguous(
                        p.cands
                            .iter()
                            .map(|c| {
                                Btn::new(
                                    c.as_str(),
                                    Intent::Do(Command::Choose {
                                        key: key.clone(),
                                        repo: p.repo.clone(),
                                        branch: c.clone(),
                                    }),
                                )
                            })
                            .collect(),
                    )
                } else if let Some(name) = &p.chosen {
                    BranchCell::Chosen {
                        name: name.clone(),
                        manual: p.manual.is_some(),
                    }
                } else {
                    BranchCell::None
                };
                RepoRowVm {
                    repo: p.repo.clone(),
                    branch,
                    pr: pr.map(|x| format!("#{} → {}", x.id, x.dst)),
                    pr_url: pr.map(|x| {
                        x.url
                            .clone()
                            .unwrap_or_else(|| pr_url(self.repo(&x.repo).host, x.id))
                    }),
                    deploy: t.landing(&p.repo).map(|l| {
                        deploy_chip_for(self.repo(&p.repo).host, &l.deploy)
                    }),
                    touched: true,
                }
            })
            .collect();
        for r in &self.repos {
            if !plan.iter().any(|p| p.repo == r.name) {
                repos.push(RepoRowVm {
                    repo: r.name.clone(),
                    branch: BranchCell::Baseline(r.base.clone()),
                    pr: None,
                    pr_url: None,
                    deploy: None,
                    touched: false,
                });
            }
        }
        Some(OverviewVm {
            key: key.clone(),
            description: t.desc.clone(),
            acceptance: t.ac.clone(),
            subtasks: t
                .subtasks
                .iter()
                .map(|(d, s)| (*d, (*s).to_string()))
                .collect(),
            repos,
            comments: t
                .comments
                .iter()
                .map(|c| CommentVm {
                    who: c.who.clone(),
                    initials: initials(&c.who),
                    at: c.at.clone(),
                    body: c.body.clone(),
                    is_new: c.n > snap && c.who != ME,
                })
                .collect(),
            attachments: Self::unreferenced_attachments(t),
            notes: t.notes.clone(),
        })
    }

    /* ---------------------------- review ---------------------------- */

    fn uat_banner(&self, t: &Ticket) -> Option<BannerVm> {
        if !Self::pre_merge_stage(t) || self.touched(t).is_empty() {
            return None;
        }
        let rs = self.uat_check(t);
        let bad: Vec<_> = rs
            .iter()
            .filter(|(_, s)| matches!(s, UatState::Conflict { .. }))
            .collect();
        let ov: Vec<_> = rs
            .iter()
            .filter(|(_, s)| matches!(s, UatState::Overlap(_)))
            .collect();
        if bad.is_empty() && ov.is_empty() {
            // Merging cleanly is the normal case: say nothing.
            return None;
        }
        let (tone, title) = if !bad.is_empty() {
            (Tone::Bad, "✕ Conflicts with uat")
        } else if !ov.is_empty() {
            (Tone::Warn, "⚠ Overlaps with tickets already on uat")
        } else {
            (Tone::Ok, "✓ Merges cleanly into uat")
        };
        let show_all = bad.is_empty() && ov.is_empty();
        let mut lines = Vec::new();
        for (repo, s) in &rs {
            match s {
                UatState::OnUat => {}
                UatState::Conflict { with, files } => lines.push(format!(
                    "{repo}: conflict with {with} in {}. A merge would fail.",
                    files.join(", ")
                )),
                UatState::Overlap(list) => lines.push(format!(
                    "{repo}: {}. Not a conflict; check the combined behaviour.",
                    list.iter()
                        .map(|(k, f)| format!("{k} also changes {f}"))
                        .collect::<Vec<_>>()
                        .join("; ")
                )),
                UatState::Clean => {
                    if show_all {
                        lines.push(format!("{repo}: clean"));
                    }
                }
            }
        }
        let mut actions = Vec::new();
        if !bad.is_empty() {
            if let Some(at) = self.conflict_sent(t) {
                lines.push(format!("Comment sent {at}."));
            } else {
                actions.push(
                    Btn::new(
                        "Comment on ticket…",
                        Intent::Do(Command::ConflictComment {
                            key: t.key.clone(),
                            also_return: false,
                        }),
                    )
                    .primary(),
                );
            }
        }
        Some(BannerVm {
            tone,
            title: title.to_string(),
            lines,
            actions,
        })
    }

    pub(crate) fn vm_review(&self, key: &TicketKey, sel: &ReviewSel) -> Option<ReviewVm> {
        let t = self.tk(key)?;
        let uat = self.uat_banner(t);
        let empty_vm = |uat: Option<BannerVm>| ReviewVm {
            key: key.clone(),
            pr_id: PrNumber(0),
            pr_url: None,
            uat,
            stale: None,
            since_toggle: None,
            prs: Vec::new(),
            diff_mode: sel.mode,
            pr: None,
            groups: Vec::new(),
            file_path: String::new(),
            hunks: Vec::new(),
            thread_count: 0,
            mark_reviewed: Btn::new("Mark reviewed", Intent::Noop)
                .disabled(Some("No pull requests".into())),
            request_changes: Btn::new("Request changes…", Intent::Noop)
                .disabled(Some("No pull requests".into())),
            approve: Btn::new("Approve…", Intent::Noop).disabled(Some("No pull requests".into())),
            note: String::new(),
            composer: None,
            empty: true,
        };
        if t.prs.is_empty() {
            return Some(empty_vm(uat));
        }
        let pr = sel
            .pr
            .and_then(|id| t.prs.iter().find(|p| p.id == id))
            .unwrap_or(&t.prs[0]);
        let stale = Self::stale_review(t);
        let since = stale && sel.since == SinceMode::Since && pr.since.is_some();
        let files: &[FileDiff] = if since {
            &pr.since.as_ref()?.files
        } else {
            &pr.files
        };
        if files.is_empty() {
            return Some(empty_vm(uat));
        }
        let fi = sel.file.min(files.len() - 1);
        let file = &files[fi];

        let groups: Vec<FileGroupVm> = t
            .prs
            .iter()
            .filter_map(|p| {
                let fl: &[FileDiff] = if since {
                    p.since.as_ref().map_or(&[][..], |s| &s.files[..])
                } else {
                    &p.files
                };
                if fl.is_empty() {
                    return None;
                }
                Some(FileGroupVm {
                    label: format!("{} #{}", p.repo, p.id),
                    files: fl
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            let hunks = hunks_of(f);
                            let nv = hunks
                                .iter()
                                .enumerate()
                                .filter(|(hi, _)| {
                                    sel.viewed.contains(&HunkId {
                                        ticket: key.clone(),
                                        pr: p.id,
                                        since: false,
                                        path: f.path.clone(),
                                        hunk: *hi,
                                    })
                                })
                                .count();
                            FileRowVm {
                                path: f.path.clone(),
                                adds: f.adds,
                                dels: f.dels,
                                progress: (!since).then(|| format!("{nv}/{}", hunks.len())),
                                viewed_all: !since && nv == hunks.len(),
                                selected: p.id == pr.id && i == fi,
                                select: Intent::SelectFile {
                                    key: key.clone(),
                                    pr: p.id,
                                    index: i,
                                },
                            }
                        })
                        .collect(),
                })
            })
            .collect();

        let threads: Vec<&Thread> = if since {
            Vec::new()
        } else {
            pr.threads
                .iter()
                .filter(|th| th.file == file.path)
                .collect()
        };
        let composer = sel
            .composer
            .as_ref()
            .filter(|(id, path, _)| !since && *id == pr.id && *path == file.path)
            .map(|(_, path, anchor)| (path.clone(), *anchor));
        let is_composer = |anchor: LineAnchor| composer.as_ref().is_some_and(|(_, a)| *a == anchor);
        let threads_at = |anchor: LineAnchor| -> Vec<ThreadVm> {
            threads
                .iter()
                .filter(|th| th.line == Some(anchor))
                .map(|th| thread_vm(th))
                .collect()
        };

        let hunks = hunks_of(file);
        let n_hunks = hunks.len();
        let hunk_vms: Vec<HunkVm> = hunks
            .iter()
            .enumerate()
            .map(|(hi, (a, b))| {
                let ls = &file.lines[*a..*b];
                let id = HunkId {
                    ticket: key.clone(),
                    pr: pr.id,
                    since,
                    path: file.path.clone(),
                    hunk: hi,
                };
                let viewed = sel.viewed.contains(&id);
                let rows = if viewed {
                    Vec::new()
                } else if sel.mode == DiffMode::Split {
                    split_rows(ls)
                        .into_iter()
                        .map(|(l, r)| {
                            let anchor = r.or(l).map_or(LineAnchor::New(0), anchor_of);
                            DiffRow::Pair {
                                left: l.map(line_vm),
                                right: r.map(line_vm),
                                threads: threads_at(anchor),
                                composer: is_composer(anchor),
                            }
                        })
                        .collect()
                } else {
                    ls.iter()
                        .map(|l| {
                            let anchor = anchor_of(l);
                            DiffRow::Line {
                                line: line_vm(l),
                                threads: threads_at(anchor),
                                composer: is_composer(anchor),
                            }
                        })
                        .collect()
                };
                HunkVm {
                    index: hi,
                    of: n_hunks,
                    adds: ls.iter().filter(|l| l.k == Kind::Add).count(),
                    dels: ls.iter().filter(|l| l.k == Kind::Del).count(),
                    viewed,
                    toggle: Intent::ToggleViewed { id },
                    rows,
                }
            })
            .collect();

        let stale_banner = stale.then(|| BannerVm {
            tone: Tone::Warn,
            title: format!(
                "⚠ New commits arrived since you reviewed this ticket{}.",
                pr.since
                    .as_ref()
                    .map_or(String::new(), |s| format!(": {} {}", s.commit, s.msg))
            ),
            lines: Vec::new(),
            actions: vec![Btn::new(
                "Start review again",
                Intent::Do(Command::StartReview(key.clone())),
            )],
        });
        let since_toggle = (stale && pr.since.is_some()).then(|| {
            (
                Btn::new(
                    "Since your review",
                    Intent::SetSinceMode {
                        key: key.clone(),
                        mode: SinceMode::Since,
                    },
                ),
                Btn::new(
                    "Full PR",
                    Intent::SetSinceMode {
                        key: key.clone(),
                        mode: SinceMode::Full,
                    },
                ),
                since,
            )
        });

        let me_approved = pr.reviewers.iter().any(|r| r.name == ME && r.approved);
        let signed = t.jira.is_signed_off();
        let approve_why = if me_approved {
            Some("You approved this PR.".to_string())
        } else if !signed {
            Some(format!(
                "Approval unlocks when the ticket reaches a signed-off status (now: {}).",
                t.jira_name.as_deref().unwrap_or(t.jira.label())
            ))
        } else if !self.gh_ready {
            Some("gh is signed out.".to_string())
        } else {
            None
        };
        let reviewed_now = t.reviewed() && !stale;
        Some(ReviewVm {
            key: key.clone(),
            pr_id: pr.id,
            pr_url: Some(
                pr.url
                    .clone()
                    .unwrap_or_else(|| pr_url(self.repo(&pr.repo).host, pr.id)),
            ),
            uat,
            stale: stale_banner,
            since_toggle,
            prs: t
                .prs
                .iter()
                .map(|p| {
                    (
                        format!("{} #{}", p.repo, p.id),
                        p.id == pr.id,
                        Intent::SelectPr {
                            key: key.clone(),
                            pr: p.id,
                        },
                    )
                })
                .collect(),
            diff_mode: sel.mode,
            pr: Some(PrHeadVm {
                title: pr.title.clone(),
                source: pr.src.to_string(),
                url: Some(
                    pr.url
                        .clone()
                        .unwrap_or_else(|| pr_url(self.repo(&pr.repo).host, pr.id)),
                ),
                dest: {
                    let cfg = self.repo(&pr.repo);
                    Badge::new(
                        pr.dst.as_str(),
                        if pr.dst == cfg.prod && cfg.prod != cfg.base {
                            Tone::Hot
                        } else {
                            Tone::Neutral
                        },
                    )
                },
                reviewers: pr
                    .reviewers
                    .iter()
                    .map(|r| (r.name.clone(), r.approved))
                    .collect(),
            }),
            groups,
            file_path: file.path.clone(),
            hunks: hunk_vms,
            thread_count: t.prs.iter().map(|p| p.threads.len()).sum(),
            mark_reviewed: Btn::new(
                if reviewed_now {
                    "✓ Reviewed"
                } else {
                    "Mark reviewed"
                },
                Intent::Do(Command::MarkReviewed(key.clone())),
            )
            .primary_if(!reviewed_now)
            .disabled(reviewed_now.then(|| "Already reviewed".to_string())),
            request_changes: Btn::new(
                "Request changes…",
                Intent::RequestChangesFrom {
                    key: key.clone(),
                    pr: pr.id,
                },
            ),
            approve: Btn::new(
                "Approve…",
                Intent::Do(Command::Approve {
                    key: key.clone(),
                    repo: pr.repo.clone(),
                    pr: pr.id,
                }),
            )
            .primary()
            .disabled(approve_why.clone()),
            note: approve_why.unwrap_or_default(),
            composer,
            empty: false,
        })
    }

    /* ---------------------------- test ---------------------------- */

    pub(crate) fn vm_test(&self, key: &TicketKey) -> Option<TestVm> {
        let t = self.tk(key)?;
        let st = t.local();
        if st != Some(Local::Active) {
            let why = match st {
                None => "Claim and review this ticket first.",
                Some(Local::Integrated) => {
                    "This ticket is already on uat. Activate it again only for a fix after alpha."
                }
                _ if !t.reviewed() => "Finish the review, then activate to test locally.",
                _ => {
                    "Activating switches the touched repos to the ticket branches, stashes local changes, and applies the test overlay where needed."
                }
            };
            let plan = self.activation_plan(t, None);
            let rows = plan
                .rows
                .iter()
                .map(|r| {
                    let was = self.ws.branches.get(&r.repo).cloned().unwrap_or_default();
                    EnvRow {
                        repo: r.repo.clone(),
                        role: if r.ticket_role {
                            Badge::new("ticket branch", Tone::Accent)
                        } else {
                            Badge::new("baseline", Tone::Neutral)
                        },
                        branch: r.branch.clone(),
                        note: format!(
                            "was {was}{}",
                            if self.ws.dirty.contains(&r.repo) {
                                ", local changes stashed"
                            } else {
                                ""
                            }
                        ),
                    }
                })
                .collect();
            let can = matches!(
                st,
                Some(Local::Claimed | Local::Reviewing | Local::Parked | Local::Integrated)
            );
            return Some(TestVm::Inactive {
                why: why.to_string(),
                errors: plan.errors,
                plan: rows,
                overlay: plan.overlay.map(|o| {
                    format!(
                        "Test overlay in {}: composer path repository → ../{}, constraint “*”. Reverted when you park or finish.",
                        o.repo, o.provider
                    )
                }),
                activate: can.then(|| {
                    Btn::new(
                        "Activate…",
                        Intent::Do(Command::Activate {
                            key: key.clone(),
                            baseline: None,
                        }),
                    )
                    .primary()
                }),
            });
        }
        let act = t.act()?;
        let mut env = Vec::new();
        for r in &act.records {
            env.push(EnvRow {
                repo: r.repo.clone(),
                role: if r.ticket_role {
                    Badge::new("ticket", Tone::Accent)
                } else {
                    Badge::new("baseline", Tone::Neutral)
                },
                branch: r.branch.clone(),
                note: r
                    .label
                    .as_ref()
                    .map_or(String::new(), |l| format!("stash “{l}”, was on {}", r.prev)),
            });
        }
        Some(TestVm::Active {
            checklist: t.checklist.clone(),
            done: t.checklist.iter().filter(|c| c.1).count(),
            notes: t.notes.clone(),
            env,
            overlay: act.overlay.as_ref().map(|o| {
                format!(
                    "Test overlay applied in {}: composer path repository → ../{}, constraint “*”. Rebuild ran. Never pushed: reverted when you park or finish, and blocked by a guard if it reaches a commit.",
                    o.repo, o.provider
                )
            }),
            actions: vec![
                Btn::new("Prepare integration →", Intent::go_ticket(key, TicketTab::Ship)).primary(),
                Btn::new("Park…", Intent::Do(Command::Park(key.clone()))),
            ],
        })
    }

    /* ---------------------------- ship ---------------------------- */

    pub(crate) fn vm_ship(&self, key: &TicketKey) -> Option<ShipVm> {
        let t = self.tk(key)?;
        let st = t.local();
        let pushed_all = self.pushed_all(t);
        let integrate_state = if st == Some(Local::Active) {
            StepState::Now
        } else if pushed_all {
            StepState::Done
        } else {
            StepState::Todo
        };
        let integrate = if st == Some(Local::Active) {
            let prep = t.prep().as_ref().map(|p| {
                p.rows
                    .iter()
                    .map(|r| match &r.outcome {
                        PrepOutcome::Ready {
                            commits,
                            files,
                            uat_before,
                            merge,
                            overlaps,
                            ..
                        } => PrepRow::Ready {
                            repo: r.repo.clone(),
                            commits: *commits,
                            files: *files,
                            uat_before: uat_before.clone(),
                            merge: merge.clone(),
                            overlaps: overlaps
                                .iter()
                                .map(|(k, f)| OverlapVm {
                                    text: format!("{k} ({f})"),
                                    tickets: vec![k.clone()],
                                })
                                .collect(),
                        },
                        PrepOutcome::AlreadyPushed { commit } => PrepRow::AlreadyPushed {
                            repo: r.repo.clone(),
                            commit: commit.clone(),
                        },
                        PrepOutcome::Conflict { files, with } => {
                            let sent = self.conflict_sent(t);
                            PrepRow::Conflict {
                                repo: r.repo.clone(),
                                with: with.clone(),
                                files: files.join(", "),
                                comment: sent.is_none().then(|| {
                                    Btn::new(
                                        "Comment on ticket…",
                                        Intent::Do(Command::ConflictComment {
                                            key: key.clone(),
                                            also_return: false,
                                        }),
                                    )
                                    .primary()
                                }),
                                sent,
                            }
                        }
                        PrepOutcome::Blocked { reason } => PrepRow::Blocked {
                            repo: r.repo.clone(),
                            reason: reason.clone(),
                        },
                    })
                    .collect::<Vec<_>>()
            });
            let items = self.prepush_items(t);
            let mut groups: Vec<PrePushGroup> = Vec::new();
            for (repo, i, text, done) in items {
                let intent = Intent::Do(Command::TogglePrePush {
                    key: key.clone(),
                    repo: repo.clone(),
                    index: i,
                });
                match groups.iter_mut().find(|g| g.repo == repo) {
                    Some(g) => g.items.push((text, done, intent)),
                    None => groups.push(PrePushGroup {
                        repo,
                        items: vec![(text, done, intent)],
                    }),
                }
            }
            let missing = self.prepush_missing(t);
            IntegrateBody::Active {
                prepare: Btn::new(
                    if prep.is_some() {
                        "Refresh preparation"
                    } else {
                        "Prepare integration"
                    },
                    Intent::Do(Command::Prepare(key.clone())),
                )
                .primary_if(prep.is_none()),
                push: Btn::new(
                    "Review & push to uat…",
                    Intent::Do(Command::PushUat(key.clone())),
                )
                .primary()
                .disabled((!self.prep_pushable(t)).then(|| {
                    if !missing.is_empty() && Self::prep_ready(t) {
                        "Tick the pre-push checks first".to_string()
                    } else {
                        "Prepare the integration first".to_string()
                    }
                })),
                prep,
                prepush: groups,
            }
        } else if pushed_all {
            IntegrateBody::Pushed {
                rows: t
                    .landings()
                    .iter()
                    .map(|l| (l.repo.clone(), l.commit.clone(), l.at.clone()))
                    .collect(),
                new_commits: t.new_commits,
            }
        } else {
            IntegrateBody::Inactive(
                "Activate and test the ticket first. Integration needs the ticket to be active."
                    .to_string(),
            )
        };

        let fails = Self::failed_repos(t);
        let runs: Vec<RunRow> = t
            .landings()
            .iter()
            .map(|l| {
                let d = &l.deploy;
                RunRow {
                    repo: l.repo.clone(),
                    chip: deploy_chip_for(self.repo(&l.repo).host, d),
                    rerun: (d.state == DeployState::Failed).then(|| {
                        Btn::new(
                            "Re-run…",
                            Intent::Do(Command::Rerun {
                                key: key.clone(),
                                repo: l.repo.clone(),
                            }),
                        )
                    }),
                    failure: match (&d.state, &d.log) {
                        (DeployState::Failed, Some(log)) => Some((
                            d.step.clone(),
                            d.run,
                            log[log.len().saturating_sub(5)..].join("\n"),
                        )),
                        _ => None,
                    },
                }
            })
            .collect();
        let dep = self.all_deployed(t);
        let runs_state = if t.landings().is_empty() {
            StepState::Todo
        } else if !fails.is_empty() {
            StepState::Bad
        } else if dep {
            StepState::Done
        } else {
            StepState::Now
        };
        let uat_moved = t
            .landings()
            .iter()
            .find_map(|l| l.deploy.uat_moved.clone())
            .map(|u| UatMovedVm {
                ticket: u.ticket,
                by: u.by,
                at: u.at,
            });

        // While the ticket is active the integration is still in progress (a first merge or a re-merge), so
        // whatever an earlier landing announced must not be acted on until the new push has deployed.
        let hold = (st == Some(Local::Active)).then(|| REMERGE_FIRST.to_string());
        let d = Self::draft(t);
        let posted = d.is_some_and(|d| d.posted);
        let moved = t.jira == JiraStatus::AlphaTesting || t.jira.is_signed_off();
        let announce = match d {
            Some(d) if d.posted => AnnounceBody::Posted {
                text: d.body.clone(),
                moved: moved.then(|| jira_badge_of(t)),
                transition: (!moved).then(|| {
                    Btn::new(
                        format!("Move to {}…", JiraStatus::AlphaTesting.label()),
                        Intent::Do(Command::Transition(key.clone())),
                    )
                    .primary()
                    .disabled(hold.clone())
                }),
            },
            Some(d) => AnnounceBody::Draft {
                id: d.id.clone(),
                text: d.body.clone(),
                post: Btn::new(
                    format!("Post and move to {}…", JiraStatus::AlphaTesting.label()),
                    Intent::Do(Command::PostAndMove {
                        key: key.clone(),
                        draft: d.id.clone(),
                    }),
                )
                .primary()
                .disabled(hold.clone()),
            },
            None if dep => AnnounceBody::Compose(
                Btn::new(
                    "Draft the comment",
                    Intent::Do(Command::ComposeDraft(key.clone())),
                )
                .primary()
                .disabled(hold.clone()),
            ),
            None => AnnounceBody::Unavailable,
        };
        let announce_state = if posted && moved {
            StepState::Done
        } else if hold.is_some() {
            StepState::Todo
        } else if d.is_some() || dep {
            StepState::Now
        } else {
            StepState::Todo
        };

        let signed = t.jira.is_signed_off();
        let pending: Vec<&Pr> = t
            .prs
            .iter()
            .filter(|p| !p.reviewers.iter().any(|r| r.name == ME && r.approved))
            .collect();
        let after_text = if t.landings().is_empty() {
            "Later.".to_string()
        } else {
            format!(
                "Alpha and UAT are done by others. When Jira reaches a signed-off status ({}), approval unlocks. New commits suggest a re-merge; a returned ticket only matters if you are mentioned.",
                JiraStatus::Done.label()
            )
        };
        let after = if !t.landings().is_empty() && signed {
            t.prs
                .iter()
                .map(|p| {
                    let ok = p.reviewers.iter().any(|r| r.name == ME && r.approved);
                    AfterRow {
                        label: format!("{} #{}", p.repo, p.id),
                        url: Some(
                            p.url
                                .clone()
                                .unwrap_or_else(|| pr_url(self.repo(&p.repo).host, p.id)),
                        ),
                        badge: if ok {
                            Badge::new("approved by you", Tone::Ok)
                        } else {
                            Badge::new("needs your approval", Tone::Warn)
                        },
                        approve: (!ok).then(|| {
                            Btn::new(
                                "Approve…",
                                Intent::Do(Command::Approve {
                                    key: key.clone(),
                                    repo: p.repo.clone(),
                                    pr: p.id,
                                }),
                            )
                            .primary()
                        }),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(ShipVm {
            key: key.clone(),
            integrate_state,
            integrate,
            runs_state,
            runs,
            uat_moved,
            announce_state,
            announce,
            after_state: if signed && !t.landings().is_empty() {
                StepState::Now
            } else {
                StepState::Todo
            },
            after_text,
            approve_all: (signed && !t.landings().is_empty() && pending.len() > 1).then(|| {
                Btn::new(
                    format!("Approve all {}…", pending.len()),
                    Intent::Do(Command::ApproveAll(key.clone())),
                )
                .primary()
            }),
            after,
        })
    }
}

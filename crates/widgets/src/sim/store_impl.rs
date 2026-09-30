//! `Store` for the simulation: builds every view model and routes every command.

use super::Sim;
use super::detail::{deploy_chip, jira_badge, local_badge, priority_tone};
use super::model::*;
use super::rules::{Sug, natural_key};
use crate::store::{Outcome, ReviewSel, Store};
use crate::vm::*;

fn act_label(i: &Intent) -> String {
    match i {
        Intent::Do(c) => match c {
            Command::Sync => "Sync now",
            Command::Claim(_) => "Claim",
            Command::StartReview(_) => "Start review",
            Command::MarkReviewed(_) => "Mark reviewed",
            Command::Activate { .. } => "Activate…",
            Command::Prepare(_) => "Prepare integration",
            Command::PushUat(_) => "Review & push to uat…",
            Command::ComposeDraft(_) => "Draft comment",
            Command::PostAndMove { .. } => "Review & post…",
            Command::Transition(_) => "Transition…",
            Command::Approve { .. } => "Approve…",
            Command::ApproveAll(_) => "Approve all…",
            Command::BreakLock(_) => "Remove lock…",
            Command::ReturnMissingPr(_) | Command::ReturnThreads(_) => "Return with comment…",
            Command::AcceptThreads(_) => "Proceed anyway…",
            Command::ConflictComment { .. } => "Comment on ticket…",
            Command::Rerun { .. } => "Re-run…",
            Command::Park(_) => "Park…",
            Command::Reclaim(_) => "Claim again",
            _ => "Do",
        },
        Intent::Go(Route::Settings) => "Open Settings",
        _ => "Open",
    }
    .to_string()
}

fn cmd(c: Command) -> Intent {
    Intent::Do(c)
}

fn audit_row(a: &AuditEntry) -> AuditRow {
    AuditRow {
        at: a.at.clone(),
        action: a.action.clone(),
        repo: a.repo.clone().unwrap_or_default(),
        ticket: a.ticket.clone(),
        outcome: Badge::new(
            a.outcome,
            match a.outcome {
                "success" => Tone::Ok,
                "failure" => Tone::Bad,
                _ => Tone::Neutral,
            },
        ),
        details: a.details.clone(),
    }
}

impl Sim {
    fn card(&self, s: &Sug) -> SuggestionCard {
        let primary = Btn::new(act_label(&s.act), s.act.clone())
            .primary_if(s.level == Level::External || !s.info);
        SuggestionCard {
            id: s.id.clone(),
            rank: s.rank,
            hotfix: s.hotfix,
            info: s.info,
            ticket: s.ticket.clone(),
            title: s.title.clone(),
            reason: s.reason.clone(),
            level: s.level,
            state: s.state,
            primary,
            alt: s.alt.clone().map(|(l, i)| Btn::new(l, i)),
            dismiss: Btn::new("Dismiss", cmd(Command::Dismiss(s.id.clone()))),
            snooze: [2u32, 10]
                .iter()
                .map(|m| {
                    Btn::new(
                        format!("{m}m"),
                        cmd(Command::Snooze {
                            id: s.id.clone(),
                            minutes: *m,
                        }),
                    )
                })
                .collect(),
            undo: Btn::new("Undo", cmd(Command::Undo(Undo::Response(s.id.clone())))),
        }
    }

    fn sorted_group(&self, g: Group) -> Vec<&Ticket> {
        let mut v: Vec<&Ticket> = self
            .tickets
            .iter()
            .filter(|t| Self::in_group(t, g))
            .collect();
        v.sort_by(|a, b| {
            a.priority
                .rank()
                .cmp(&b.priority.rank())
                .then_with(|| natural_key(&a.key).cmp(&natural_key(&b.key)))
        });
        v
    }

    fn ticket_row(&self, t: &Ticket) -> TicketRowVm {
        let un = Self::unseen_count(t);
        let mut flags = Vec::new();
        if !t.merges.is_empty() {
            flags.push(if self.all_deployed(t) {
                Badge::new("deployed", Tone::Ok)
            } else if !Self::failed_repos(t).is_empty() {
                Badge::new("failed", Tone::Bad)
            } else {
                Badge::new("deploying", Tone::Warn)
            });
        }
        if un > 0 {
            flags.push(Badge::new(format!("{un} new"), Tone::Accent));
        }
        if Self::stale_review(t) {
            flags.push(Badge::new("new commits", Tone::Warn));
        }
        let bt = Self::blocking_threads(t).len();
        if bt > 0 {
            flags.push(Badge::new(
                format!("{bt} unresolved"),
                if self.th_wait_expired(t) {
                    Tone::Bad
                } else {
                    Tone::Warn
                },
            ));
        }
        if self.pr_gap(t).is_some() {
            flags.push(Badge::new(
                "no PR",
                if self.pr_wait_expired(t) {
                    Tone::Bad
                } else {
                    Tone::Warn
                },
            ));
        }
        if let Some(f) = self.uat_flag(t) {
            flags.push(f);
        }
        TicketRowVm {
            key: t.key.clone(),
            priority: Badge::new(t.priority.label(), priority_tone(t.priority)),
            title: t.title.clone(),
            hotfix: self.is_hotfix(t),
            sub: format!("{} · {}", t.kind, t.assignee),
            jira: jira_badge(&t.jira),
            local: local_badge(t.local),
            repos: t.prs.iter().map(|p| p.repo.clone()).collect(),
            flags,
        }
    }

    fn activity(&self, key: Option<&str>, n: usize) -> Vec<RightRow> {
        let rows: Vec<RightRow> = self
            .audit
            .iter()
            .filter(|a| key.is_none_or(|k| a.ticket.as_deref() == Some(k)))
            .take(n)
            .map(|a| RightRow::Activity {
                at: a.at.clone(),
                text: match &a.repo {
                    Some(r) => format!("{} · {r}", a.action),
                    None => a.action.clone(),
                },
                failed: a.outcome == "failure",
            })
            .collect();
        if rows.is_empty() {
            vec![RightRow::Muted("No activity yet.".to_string())]
        } else {
            rows
        }
    }

    fn sync_section(&self) -> Option<RightSection> {
        if self.sync.report.is_empty() {
            return None;
        }
        Some(RightSection {
            title: "Last sync".to_string(),
            rows: self
                .sync
                .report
                .iter()
                .map(|(ok, src, text)| RightRow::Report {
                    ok: *ok,
                    source: src.clone(),
                    text: text.clone(),
                })
                .collect(),
        })
    }

    fn right_next(&self) -> Vec<RightSection> {
        let mut out = Vec::new();
        if let Some(t) = self.active_key().and_then(|k| self.tk(&k)) {
            let done = t.checklist.iter().filter(|c| c.1).count();
            out.push(RightSection {
                title: "Now".to_string(),
                rows: vec![
                    RightRow::Item {
                        head: Vec::new(),
                        key: Some(t.key.clone()),
                        title: t.title.clone(),
                        sub: Some(format!(
                            "{} · {done}/{} checked",
                            Self::fmt_dur(self.duration_ms(t)),
                            t.checklist.len()
                        )),
                    },
                    RightRow::Button(Btn::new("Open", Intent::go_ticket(&t.key, TicketTab::Test))),
                ],
            });
        }
        let today: Vec<&Ticket> = self
            .tickets
            .iter()
            .filter(|t| self.duration_ms(t) > 0)
            .collect();
        if !today.is_empty() {
            let total: i64 = today.iter().map(|t| self.duration_ms(t)).sum();
            let mut rows = vec![RightRow::Kv(Kv::text("Total", Self::fmt_dur(total)))];
            rows.extend(
                today
                    .iter()
                    .map(|t| RightRow::Kv(Kv::text(&t.key, Self::fmt_dur(self.duration_ms(t))))),
            );
            out.push(RightSection {
                title: "Today".to_string(),
                rows,
            });
        }
        out.extend(self.sync_section());
        let queue: Vec<RightRow> = self
            .sorted_group(Group::Pool)
            .into_iter()
            .take(4)
            .map(|t| {
                let mut head = vec![Badge::new(t.priority.label(), priority_tone(t.priority))];
                if self.is_hotfix(t) {
                    head.push(Badge::new("HOTFIX", Tone::Hot));
                }
                RightRow::Item {
                    head,
                    key: Some(t.key.clone()),
                    title: t.title.clone(),
                    sub: None,
                }
            })
            .collect();
        out.push(RightSection {
            title: "Claim queue".to_string(),
            rows: if queue.is_empty() {
                vec![RightRow::Muted("Empty.".to_string())]
            } else {
                queue
            },
        });
        out.push(RightSection {
            title: "Recent activity".to_string(),
            rows: self.activity(None, 6),
        });
        out
    }

    fn right_ticket(&self, t: &Ticket) -> Vec<RightSection> {
        let hot = self.is_hotfix(t);
        let mut out = vec![RightSection {
            title: "Status".to_string(),
            rows: vec![
                RightRow::Kv(Kv::badges("Jira", vec![jira_badge(&t.jira)])),
                RightRow::Kv(Kv::badges(
                    "Local",
                    vec![
                        local_badge(t.local)
                            .unwrap_or_else(|| Badge::new("not claimed", Tone::Neutral)),
                    ],
                )),
                RightRow::Kv(Kv::badges(
                    "Kind",
                    vec![if hot {
                        Badge::new("HOTFIX", Tone::Hot)
                    } else {
                        Badge::new("Normal", Tone::Neutral)
                    }],
                )),
                RightRow::Kv(Kv::text("Time", Self::fmt_dur(self.duration_ms(t)))),
                RightRow::Kv(Kv::badges(
                    "Reviewed",
                    vec![if t.reviewed {
                        Badge::new("yes", Tone::Ok)
                    } else {
                        Badge::new("no", Tone::Neutral)
                    }],
                )),
            ],
        }];
        let badges = |v: &[&str]| -> Vec<Badge> {
            if v.is_empty() {
                vec![Badge::new("–", Tone::Neutral)]
            } else {
                v.iter().map(|x| Badge::new(*x, Tone::Neutral)).collect()
            }
        };
        out.push(RightSection {
            title: "Details".to_string(),
            rows: vec![
                RightRow::Kv(Kv::text("Type", t.kind)),
                RightRow::Kv(Kv::badges(
                    "Priority",
                    vec![Badge::new(t.priority.label(), priority_tone(t.priority))],
                )),
                RightRow::Kv(Kv::text("Assignee", t.assignee)),
                RightRow::Kv(Kv::text("Reporter", t.reporter)),
                RightRow::Kv(Kv::text("Sprint", t.sprint)),
                RightRow::Kv(Kv::text("Epic", t.epic)),
                RightRow::Kv(Kv::text("Fix version", t.fix_version)),
                RightRow::Kv(Kv::text("Estimate", t.estimate)),
                RightRow::Kv(Kv::badges("Components", badges(&t.components))),
                RightRow::Kv(Kv::badges("Labels", badges(&t.labels))),
                RightRow::Kv(Kv::text("Created", t.created)),
                RightRow::Kv(Kv::text("Updated", t.updated)),
            ],
        });
        let prs: Vec<RightRow> = t
            .prs
            .iter()
            .map(|p| {
                let cfg = self.repo(&p.repo);
                let ap = p.reviewers.iter().filter(|r| r.approved).count();
                let d = t.deploy.get(&p.repo);
                let mut sub = format!(
                    "{} → {} · {ap}/{} approvals",
                    p.src,
                    p.dst,
                    p.reviewers.len()
                );
                if let Some(d) = d {
                    sub.push_str(&format!(" · run #{} {}", d.run, d.state.word()));
                }
                RightRow::Item {
                    head: vec![Badge::new(
                        format!("{} #{}", p.repo, p.id),
                        if p.dst == cfg.prod && cfg.prod != cfg.base {
                            Tone::Hot
                        } else {
                            Tone::Neutral
                        },
                    )],
                    key: None,
                    title: p.title.clone(),
                    sub: Some(sub),
                }
            })
            .collect();
        out.push(RightSection {
            title: "Pull requests".to_string(),
            rows: if prs.is_empty() {
                vec![RightRow::Muted(if self.pr_gap(t).is_some() {
                    self.gap_text(t)
                } else {
                    "None.".to_string()
                })]
            } else {
                prs
            },
        });
        if !t.links.is_empty() {
            out.push(RightSection {
                title: "Linked issues".to_string(),
                rows: t
                    .links
                    .iter()
                    .map(|l| RightRow::Item {
                        head: vec![Badge::new(l.rel, Tone::Neutral), jira_badge(l.status)],
                        key: self.tk(l.key).map(|_| l.key.to_string()),
                        title: format!("{} {}", l.key, l.title),
                        sub: None,
                    })
                    .collect(),
            });
        }
        out.push(RightSection {
            title: "Activity".to_string(),
            rows: self.activity(Some(&t.key), 8),
        });
        out
    }

    fn stale_label(&self, who: &str) -> String {
        format!("Cannot send: {who} is signed out.")
    }

    fn blocked_for(&self, c: &Command) -> Option<String> {
        let acli = matches!(
            c,
            Command::PostAndMove { .. }
                | Command::Transition(_)
                | Command::PostComment { .. }
                | Command::ReturnMissingPr(_)
                | Command::ReturnThreads(_)
                | Command::ConflictComment { .. }
        );
        let gh = matches!(
            c,
            Command::PushUat(_)
                | Command::Rerun { .. }
                | Command::Approve { .. }
                | Command::ApproveAll(_)
                | Command::RequestChanges { .. }
                | Command::InlineComment { .. }
        );
        if acli && !self.jira_ready {
            return Some(format!(
                "{} Run acli jira auth login, then re-check in Settings.",
                self.stale_label("acli")
            ));
        }
        if gh && !self.gh_ready {
            return Some(format!(
                "{} Run gh auth login, then re-check in Settings.",
                self.stale_label("gh")
            ));
        }
        None
    }

    fn preview_for(&self, c: &Command) -> Option<Preview> {
        let p = |title: &str,
                 ticket: Option<&str>,
                 risk: Risk,
                 summary: &str,
                 payload: Vec<String>,
                 facts: Vec<String>,
                 confirm: &str| Preview {
            title: title.to_string(),
            ticket: ticket.map(str::to_string),
            risk,
            summary: summary.to_string(),
            payload,
            facts,
            type_key: None,
            confirm_label: confirm.to_string(),
            blocked: self.blocked_for(c),
        };
        match c {
            Command::PushUat(key) => {
                let t = self.tk(key)?;
                let prep = t.prep.as_ref()?;
                let mut payload = Vec::new();
                let mut facts = Vec::new();
                for r in &prep.rows {
                    match &r.outcome {
                        PrepOutcome::Ready {
                            merge,
                            uat_before,
                            commits,
                            files,
                            ..
                        } => {
                            payload.push(format!(
                                "{}   git push origin {merge}:refs/heads/uat",
                                r.repo
                            ));
                            facts.push(format!(
                                "{}   uat {uat_before} → {merge}   {commits} commits · {files} files",
                                r.repo
                            ));
                        }
                        PrepOutcome::AlreadyPushed { commit } => facts.push(format!(
                            "{}   already on uat ({commit}); nothing to push",
                            r.repo
                        )),
                        _ => {}
                    }
                }
                facts.push(
                    "Checks passed: overlay guard ✓ (by SHA) · no overlay in worktree ✓"
                        .to_string(),
                );
                let mut pv = p(
                    "Push to uat",
                    Some(key),
                    Risk::High,
                    "Pushing deploys to alpha. No force: a rejected push stops and tells you.",
                    payload,
                    facts,
                    "Push to uat",
                );
                pv.type_key = Some(key.clone());
                Some(pv)
            }
            Command::Rerun { key, repo } => {
                let d = self.tk(key)?.deploy.get(repo)?;
                Some(p(
                    "Re-run failed workflow",
                    Some(key),
                    Risk::Medium,
                    "Starts a new Actions run for the same commit. The failed run stays in the history.",
                    vec![format!(
                        "gh run rerun {} --repo {}",
                        d.run,
                        self.repo(repo).host
                    )],
                    Vec::new(),
                    "Re-run",
                ))
            }
            Command::PostAndMove { key, draft } => {
                let t = self.tk(key)?;
                let d = t.drafts.iter().find(|d| &d.id == draft)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(d.body.lines().map(|l| format!("  {l}")));
                payload.push(format!("Transition: {JIRA_REVIEW} → {JIRA_ALPHA}"));
                Some(p(
                    &format!("Post the deploy comment and move to {JIRA_ALPHA}"),
                    Some(key),
                    Risk::Medium,
                    "One confirm sends the comment and moves the ticket.",
                    payload,
                    Vec::new(),
                    "Post and move",
                ))
            }
            Command::Transition(key) => Some(p(
                &format!("Move to {JIRA_ALPHA}"),
                Some(key),
                Risk::Medium,
                "Changes the Jira status. The comment was already posted.",
                vec![format!("Transition: {JIRA_REVIEW} → {JIRA_ALPHA}")],
                Vec::new(),
                "Move ticket",
            )),
            Command::Approve { key, repo, pr } => Some(p(
                &format!("Approve {} #{pr}", self.repo(repo).host),
                Some(key),
                Risk::Medium,
                "Approval is the last gate. It cannot be withdrawn from here.",
                vec![format!(
                    "gh pr review {pr} --approve --repo {}",
                    self.repo(repo).host
                )],
                Vec::new(),
                "Approve",
            )),
            Command::ApproveAll(key) => {
                let t = self.tk(key)?;
                let payload = t
                    .prs
                    .iter()
                    .filter(|x| !x.reviewers.iter().any(|r| r.name == ME && r.approved))
                    .map(|x| {
                        format!(
                            "gh pr review {} --approve --repo {}",
                            x.id,
                            self.repo(&x.repo).host
                        )
                    })
                    .collect();
                Some(p(
                    "Approve all pull requests",
                    Some(key),
                    Risk::Medium,
                    "Approval is the last gate. It cannot be withdrawn from here.",
                    payload,
                    Vec::new(),
                    "Approve all",
                ))
            }
            Command::RequestChanges { key, pr, text } => Some(p(
                "Request changes",
                Some(key),
                Risk::Medium,
                "Posts a review that blocks the merge until it is resolved.",
                vec![format!("PR #{pr}"), format!("Body (verbatim): {text}")],
                Vec::new(),
                "Request changes",
            )),
            Command::InlineComment {
                key,
                pr,
                file,
                line,
                text,
            } => Some(p(
                "Comment on a line",
                Some(key),
                Risk::Medium,
                "Posts an inline comment. If GitHub cannot anchor it, nothing is posted instead of a general comment.",
                vec![
                    format!("PR #{pr} · {file} · line {}", line.get(1..).unwrap_or("")),
                    format!("Body (verbatim): {text}"),
                ],
                Vec::new(),
                "Post comment",
            )),
            Command::PostComment { key, text } => Some(p(
                "Post a comment to Jira",
                Some(key),
                Risk::Low,
                "Adds a comment to the ticket.",
                vec!["Body (verbatim):".to_string(), format!("  {text}")],
                Vec::new(),
                "Post comment",
            )),
            Command::ReturnMissingPr(key) => {
                let t = self.tk(key)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(self.return_body(t).lines().map(|l| format!("  {l}")));
                payload.push(format!("Transition: {JIRA_REVIEW} → {JIRA_RETURNED}"));
                Some(p(
                    &format!("Return {key}: no pull request"),
                    Some(key),
                    Risk::Medium,
                    "Comments on the ticket and sends it back to development.",
                    payload,
                    Vec::new(),
                    "Return ticket",
                ))
            }
            Command::ReturnThreads(key) => {
                let t = self.tk(key)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(
                    self.return_threads_body(t)
                        .lines()
                        .map(|l| format!("  {l}")),
                );
                payload.push(format!("Transition: {JIRA_REVIEW} → {JIRA_RETURNED}"));
                Some(p(
                    &format!("Return {key}: unresolved review comments"),
                    Some(key),
                    Risk::Medium,
                    "Comments on the ticket and sends it back to development.",
                    payload,
                    Vec::new(),
                    "Return ticket",
                ))
            }
            Command::ConflictComment { key, .. } => {
                let t = self.tk(key)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(self.conflict_body(t).lines().map(|l| format!("  {l}")));
                Some(p(
                    &format!("Tell the developer {key} conflicts with uat"),
                    Some(key),
                    Risk::Medium,
                    "Adds a comment to the ticket. The ticket stays where it is.",
                    payload,
                    Vec::new(),
                    "Post comment",
                ))
            }
            Command::AcceptThreads(key) => {
                let t = self.tk(key)?;
                let payload = Self::blocking_threads(t)
                    .iter()
                    .map(Self::thread_line)
                    .collect();
                Some(p(
                    "Proceed with unresolved comments",
                    Some(key),
                    Risk::Low,
                    "Local only. Nothing is sent. The deploy comment will list these comments.",
                    payload,
                    Vec::new(),
                    "Proceed anyway",
                ))
            }
            Command::Park(key) => Some(p(
                &format!("Park {key}"),
                Some(key),
                Risk::Low,
                "Local only. Repos go back to where they were, stashes are restored and the overlay is reverted.",
                self.tk(key)?
                    .act
                    .as_ref()
                    .map(|a| {
                        a.records
                            .iter()
                            .map(|r| {
                                format!(
                                    "{}  back on {}{}",
                                    r.repo,
                                    r.prev,
                                    if r.stash { ", stash popped" } else { "" }
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                Vec::new(),
                "Park",
            )),
            Command::BreakLock(repo) => Some(p(
                &format!("Remove the stale lock in {repo}"),
                None,
                Risk::Low,
                "Local only. Checks that no git process runs first.",
                vec![format!("rm {repo}/.git/index.lock")],
                Vec::new(),
                "Remove lock",
            )),
            Command::StopWorkspace => {
                let payload =
                    vec!["docker compose stop (dependency order: web → api → db)".to_string()];
                let facts = self
                    .ws
                    .dirty
                    .iter()
                    .map(|r| format!("{r} has uncommitted changes"))
                    .collect();
                Some(p(
                    "Stop the workspace",
                    None,
                    Risk::Medium,
                    "Stops every service. Uncommitted work stays on disk.",
                    payload,
                    facts,
                    "Stop",
                ))
            }
            _ => None,
        }
    }
}

impl Store for Sim {
    fn counts(&self) -> Counts {
        Counts {
            groups: Group::LIST
                .iter()
                .map(|g| {
                    (
                        *g,
                        self.tickets
                            .iter()
                            .filter(|t| Self::in_group(t, *g))
                            .count(),
                    )
                })
                .collect(),
            suggestions: self
                .visible_suggestions()
                .iter()
                .filter(|s| !s.info || s.level != Level::Automatic)
                .count(),
            attention: self.attention().len(),
        }
    }

    fn status(&self) -> StatusVm {
        let active = self.active_key().and_then(|k| self.tk(&k));
        let mut chips = Vec::new();
        for (repo, l) in &self.locks {
            let (what, tone) = match l.kind {
                LockKind::Stale => ("stale lock".to_string(), Tone::Bad),
                LockKind::Sync => ("fetching".to_string(), Tone::Warn),
                LockKind::External => (l.by.clone(), Tone::Warn),
            };
            chips.push(Badge::new(format!("◼ {repo}: {what}"), tone));
        }
        let (sync_text, sync_tone) = if self.sync.running {
            ("Syncing…".to_string(), Tone::Accent)
        } else if let Some(e) = &self.sync.last_error {
            (e.clone(), Tone::Warn)
        } else {
            (
                format!("Synced {}m ago", self.mins_ago(self.sync.last_ok)),
                Tone::Neutral,
            )
        };
        StatusVm {
            active: active.map(|t| ActiveChip {
                key: t.key.clone(),
                elapsed: Self::fmt_dur(self.duration_ms(t)),
            }),
            overlay: active
                .and_then(|t| t.act.as_ref())
                .and_then(|a| a.overlay.as_ref())
                .map(|o| o.repo.clone()),
            chips,
            jira_ready: self.jira_ready,
            gh_ready: self.gh_ready,
            sync_text,
            sync_tone,
            sync_running: self.sync.running,
        }
    }

    fn tab_info(&self, key: &str) -> Option<TabInfo> {
        let t = self.tk(key)?;
        Some(TabInfo {
            title: t.title.clone(),
            bad: !Self::failed_repos(t).is_empty()
                || !self.uat_conflicts(t).is_empty()
                || self.pr_wait_expired(t)
                || self.th_wait_expired(t),
            unseen: Self::unseen_count(t) > 0,
            active: t.local == Some(Local::Active),
        })
    }

    fn right_panel(&self, route: &Route) -> Vec<RightSection> {
        match route {
            Route::Ticket { key, .. } => self
                .tk(key)
                .map(|t| self.right_ticket(t))
                .unwrap_or_default(),
            Route::Next => self.right_next(),
            _ => {
                let mut out: Vec<RightSection> = self.sync_section().into_iter().collect();
                out.push(RightSection {
                    title: "Activity".to_string(),
                    rows: self.activity(None, 8),
                });
                out
            }
        }
    }

    fn palette(&self, query: &str) -> Vec<PaletteItem> {
        let q = query.trim().to_lowercase();
        let mut items = vec![
            PaletteItem {
                label: "Next".into(),
                hint: "screen".into(),
                intent: Intent::Go(Route::Next),
            },
            PaletteItem {
                label: "Sync now".into(),
                hint: "action".into(),
                intent: cmd(Command::Sync),
            },
        ];
        for g in Group::LIST {
            items.push(PaletteItem {
                label: g.label().to_string(),
                hint: "tickets".into(),
                intent: Intent::go_tickets(g),
            });
        }
        for t in &self.tickets {
            items.push(PaletteItem {
                label: format!("{} {}", t.key, t.title),
                hint: "ticket".into(),
                intent: Intent::go_ticket(&t.key, TicketTab::Overview),
            });
        }
        items.push(PaletteItem {
            label: "On uat".into(),
            hint: "screen".into(),
            intent: Intent::Go(Route::OnUat),
        });
        items.push(PaletteItem {
            label: "Workspace".into(),
            hint: "screen".into(),
            intent: Intent::Go(Route::Workspace),
        });
        items.push(PaletteItem {
            label: "Audit log".into(),
            hint: "screen".into(),
            intent: Intent::Go(Route::Audit),
        });
        items.push(PaletteItem {
            label: "Settings".into(),
            hint: "screen".into(),
            intent: Intent::Go(Route::Settings),
        });
        items
            .into_iter()
            .filter(|i| q.is_empty() || i.label.to_lowercase().contains(&q))
            .take(12)
            .collect()
    }

    fn next(&self, show_all: bool) -> NextVm {
        let all = self.suggest();
        let visible = |s: &&Sug| matches!(s.state, SugState::Open | SugState::Resurfaced);
        let hidden = all.iter().filter(|s| !visible(s)).count();
        NextVm {
            cards: all
                .iter()
                .filter(|s| show_all || visible(s))
                .map(|s| self.card(s))
                .collect(),
            hidden,
            show_all,
        }
    }

    fn attention(&self) -> AttentionVm {
        AttentionVm {
            items: self.attention().iter().map(|s| self.card(s)).collect(),
        }
    }

    fn tickets(&self, group: Group) -> TicketListVm {
        let rows = self.sorted_group(group);
        let sections = if group == Group::Returned {
            let (asked, rest): (Vec<&Ticket>, Vec<&Ticket>) = rows
                .into_iter()
                .partition(|t| !Self::unseen_mentions(t).is_empty());
            let mut s = Vec::new();
            if !asked.is_empty() {
                s.push(TicketSection {
                    heading: Some(format!("Mentions you ({})", asked.len())),
                    rows: asked.iter().map(|t| self.ticket_row(t)).collect(),
                });
            }
            if !rest.is_empty() {
                s.push(TicketSection {
                    heading: Some(format!("Returned only ({})", rest.len())),
                    rows: rest.iter().map(|t| self.ticket_row(t)).collect(),
                });
            }
            s
        } else if rows.is_empty() {
            Vec::new()
        } else {
            vec![TicketSection {
                heading: None,
                rows: rows.iter().map(|t| self.ticket_row(t)).collect(),
            }]
        };
        TicketListVm {
            group,
            tabs: Group::LIST
                .iter()
                .map(|g| {
                    (
                        *g,
                        self.tickets
                            .iter()
                            .filter(|t| Self::in_group(t, *g))
                            .count(),
                    )
                })
                .collect(),
            sections,
        }
    }

    fn ticket_exists(&self, key: &str) -> bool {
        self.tk(key).is_some()
    }

    fn ticket_title(&self, key: &str) -> String {
        self.tk(key).map(|t| t.title.clone()).unwrap_or_default()
    }

    fn ticket_head(&self, key: &str, _tab: TicketTab) -> Option<TicketHeadVm> {
        self.vm_head(key)
    }

    fn overview(&self, key: &str, seen: Option<u32>) -> Option<OverviewVm> {
        self.vm_overview(key, seen)
    }

    fn review(&self, key: &str, sel: &ReviewSel) -> Option<ReviewVm> {
        self.vm_review(key, sel)
    }

    fn test(&self, key: &str) -> Option<TestVm> {
        self.vm_test(key)
    }

    fn ship(&self, key: &str) -> Option<ShipVm> {
        self.vm_ship(key)
    }

    fn timeline(&self, key: &str) -> Vec<AuditRow> {
        self.audit
            .iter()
            .filter(|a| a.ticket.as_deref() == Some(key))
            .map(audit_row)
            .collect()
    }

    fn on_uat(&self) -> OnUatVm {
        let show_docs = self
            .tickets
            .iter()
            .any(|t| t.merges.iter().any(|m| m.repo == "docs"));
        let cols: Vec<&RepoCfg> = self
            .repos
            .iter()
            .filter(|r| r.name != "docs" || show_docs)
            .collect();
        let mut overlaps = Vec::new();
        let columns = cols
            .iter()
            .map(|r| {
                let list: Vec<&Ticket> = self
                    .tickets
                    .iter()
                    .filter(|t| t.merges.iter().any(|m| m.repo == r.name))
                    .collect();
                for i in 0..list.len() {
                    for j in (i + 1)..list.len() {
                        let paths = |t: &Ticket| -> Vec<String> {
                            t.prs
                                .iter()
                                .filter(|p| p.repo == r.name)
                                .flat_map(|p| p.files.iter().map(|f| f.path.clone()))
                                .collect()
                        };
                        let b = paths(list[j]);
                        for f in paths(list[i]).into_iter().filter(|f| b.contains(f)) {
                            overlaps.push(format!(
                                "{}: {} and {} both change {f}.",
                                r.name, list[i].key, list[j].key
                            ));
                        }
                    }
                }
                UatColumn {
                    repo: r.name.to_string(),
                    host: r.host.to_string(),
                    items: list
                        .iter()
                        .filter_map(|t| {
                            let m = t.merges.iter().find(|m| m.repo == r.name)?;
                            Some(UatItem {
                                key: t.key.clone(),
                                title: t.title.clone(),
                                commit: m.commit.clone(),
                                chip: t.deploy.get(r.name).map(deploy_chip),
                                jira: jira_badge(&t.jira),
                            })
                        })
                        .collect(),
                }
            })
            .collect();
        OnUatVm { columns, overlaps }
    }

    fn workspace(&self) -> WorkspaceVm {
        let active = self.active_key().and_then(|k| self.tk(&k));
        let rows = self
            .repos
            .iter()
            .map(|r| {
                let mut badges = vec![if self.ws.up {
                    Badge::new("up", Tone::Ok)
                } else {
                    Badge::new("down", Tone::Neutral)
                }];
                badges.push(if self.ws.dirty.contains(r.name) {
                    Badge::new("uncommitted changes", Tone::Warn)
                } else {
                    Badge::new("clean", Tone::Neutral)
                });
                if active
                    .and_then(|t| t.act.as_ref())
                    .and_then(|a| a.overlay.as_ref())
                    .is_some_and(|o| o.repo == r.name)
                {
                    badges.push(Badge::new("⚡ overlay", Tone::Warn));
                }
                let mut break_lock = None;
                if let Some(l) = self.locks.get(r.name) {
                    match l.kind {
                        LockKind::Stale => {
                            badges.push(Badge::new("stale index.lock", Tone::Bad));
                            break_lock = Some(Btn::new(
                                "Remove lock…",
                                cmd(Command::BreakLock(r.name.to_string())),
                            ));
                        }
                        _ => badges.push(Badge::new(
                            format!("busy: {}", Self::lock_text(l)),
                            Tone::Warn,
                        )),
                    }
                }
                WsRow {
                    repo: r.name.to_string(),
                    services: r.services,
                    branch: self.ws.branches.get(r.name).cloned().unwrap_or_default(),
                    badges,
                    break_lock,
                }
            })
            .collect();
        WorkspaceVm {
            name: "shop".to_string(),
            up: self.ws.up,
            order: "db → api → web".to_string(),
            rows,
            note: active.map(|t| {
                format!(
                    "Active ticket {} holds the repos above. Stashed local changes are restored when you park or finish.",
                    t.key
                )
            }),
            toggle: if self.ws.up {
                Btn::new("Stop…", cmd(Command::StopWorkspace))
            } else {
                Btn::new("Start", cmd(Command::StartWorkspace)).primary()
            },
        }
    }

    fn audit(&self) -> Vec<AuditRow> {
        self.audit.iter().map(audit_row).collect()
    }

    fn settings(&self) -> SettingsVm {
        let provider =
            |name: &str, version: &str, hint: &str, ready: bool, p: Provider| ProviderVm {
                name: name.to_string(),
                version: version.to_string(),
                ready,
                login_hint: hint.to_string(),
                toggle: Btn::new(
                    if ready {
                        "Simulate sign-out"
                    } else {
                        "Fake sign-in"
                    },
                    cmd(Command::SetProvider {
                        provider: p,
                        ready: !ready,
                    }),
                )
                .primary_if(!ready),
            };
        SettingsVm {
            providers: vec![
                provider(
                    "acli · Jira",
                    "v1.3.39",
                    "acli jira auth login",
                    self.jira_ready,
                    Provider::Jira,
                ),
                provider(
                    "gh · GitHub",
                    "v2.x",
                    "gh auth login",
                    self.gh_ready,
                    Provider::GitHub,
                ),
            ],
            wait_options: [15u32, 30, 60, 120]
                .iter()
                .map(|m| {
                    (
                        if *m < 60 {
                            format!("{m}m")
                        } else {
                            format!("{}h", m / 60)
                        },
                        self.wait_min == *m,
                        Btn::new(String::new(), cmd(Command::SetWaitMinutes(*m))),
                    )
                })
                .collect(),
            mapping: vec![
                ("Review status".into(), JIRA_REVIEW.into()),
                ("Alpha status".into(), JIRA_ALPHA.into()),
                ("Returned".into(), JIRA_RETURNED.into()),
                ("Signed off".into(), JIRA_SIGNED.join(", ")),
                (
                    "Review JQL".into(),
                    "project = PROJ AND status = \"In Review\"".into(),
                ),
                ("My account id".into(), "acct-0001".into()),
            ],
            repos: self
                .repos
                .iter()
                .map(|r| {
                    [
                        r.name.to_string(),
                        r.base.to_string(),
                        r.prod.to_string(),
                        r.consumes
                            .map_or("–".to_string(), |c| format!("consumes {c}")),
                    ]
                })
                .collect(),
            data: vec![
                ("state.db".into(), "…/de/state.db".into()),
                ("cache.db".into(), "…/de/cache.db".into()),
            ],
            reset: Btn::new("Reset this demo", cmd(Command::ResetDemo)),
        }
    }

    fn simulate(&self) -> Vec<SimGroup> {
        let b = |label: &str, on: bool, ev: SimEvent| SimButton {
            label: label.to_string(),
            on,
            intent: cmd(Command::Sim(ev)),
        };
        let web = Some("web".to_string());
        vec![
            SimGroup {
                title: "World".to_string(),
                buttons: vec![
                    b(
                        "Offline",
                        self.sims.offline,
                        SimEvent::Offline(!self.sims.offline),
                    ),
                    b(
                        "uat conflict in web",
                        self.sims.conflict.is_some(),
                        SimEvent::Conflict(if self.sims.conflict.is_some() {
                            None
                        } else {
                            web.clone()
                        }),
                    ),
                    b(
                        "Overlay leak in web",
                        self.sims.leak.is_some(),
                        SimEvent::OverlayLeak(if self.sims.leak.is_some() {
                            None
                        } else {
                            web.clone()
                        }),
                    ),
                    b(
                        "uat moves before push",
                        self.sims.uat_moved,
                        SimEvent::UatMoves(!self.sims.uat_moved),
                    ),
                    b(
                        "Deploy fails in web",
                        self.sims.fail_deploy.is_some(),
                        SimEvent::FailDeploy(if self.sims.fail_deploy.is_some() {
                            None
                        } else {
                            web
                        }),
                    ),
                ],
            },
            SimGroup {
                title: "Events".to_string(),
                buttons: vec![
                    b(
                        "Mention on PROJ-142",
                        false,
                        SimEvent::Mention("PROJ-142".into()),
                    ),
                    b(
                        "Sign off PROJ-127",
                        false,
                        SimEvent::SignOff("PROJ-127".into()),
                    ),
                    b(
                        "Return PROJ-127",
                        false,
                        SimEvent::Returned("PROJ-127".into()),
                    ),
                    b(
                        "PROJ-131 back to review",
                        false,
                        SimEvent::BackToReview("PROJ-131".into()),
                    ),
                    b(
                        "PR arrives on PROJ-163",
                        false,
                        SimEvent::PrArrives("PROJ-163".into()),
                    ),
                    b(
                        "Resolve PROJ-150 threads",
                        false,
                        SimEvent::ResolveThreads("PROJ-150".into()),
                    ),
                    b(
                        "New commits on PROJ-142",
                        false,
                        SimEvent::NewCommits("PROJ-142".into()),
                    ),
                    b(
                        "uat moves after PROJ-127",
                        false,
                        SimEvent::UatAfterPush("PROJ-127".into()),
                    ),
                    b(
                        "Stale lock in web",
                        matches!(self.locks.get("web"), Some(l) if matches!(l.kind, LockKind::Stale)),
                        SimEvent::StaleLock("web".into()),
                    ),
                    b(
                        "de CLI holds api-client",
                        matches!(self.locks.get("api-client"), Some(l) if matches!(l.kind, LockKind::External)),
                        SimEvent::ExternalLock("api-client".into()),
                    ),
                    b("Skip 10 minutes", false, SimEvent::SkipMinutes(10)),
                ],
            },
        ]
    }

    fn comments_seen(&self, key: &str) -> u32 {
        self.tk(key).map_or(0, |t| t.seen_n)
    }

    fn diagnose(&self) -> String {
        let gh = if self.gh_ready {
            "[exit 0]\ngithub.com\n  ✓ Logged in to github.com"
        } else {
            "[exit 1]\nYou are not logged into any GitHub hosts. To log in, run: gh auth login"
        };
        let jira = if self.jira_ready {
            "[exit 0]\n✓ Authenticated"
        } else {
            "[exit 1]\nNot authenticated. Run: acli jira auth login"
        };
        format!(
            "$ acli --version\n[exit 0]\nacli version 1.3.39-stable\n\n$ acli jira auth status\n{jira}\n\n$ gh --version\n[exit 0]\ngh version 2.x\n\n$ gh auth status\n{gh}"
        )
    }

    fn baseline_choices(&self, key: &str) -> BaselineVm {
        let prod: Vec<&str> = {
            let mut v: Vec<&str> = self.repos.iter().map(|r| r.prod).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        BaselineVm {
            key: key.to_string(),
            choices: vec![
                (
                    Baseline::Production,
                    format!("production ({})", prod.join(", ")),
                ),
                (Baseline::Develop, "develop".to_string()),
                (Baseline::Uat, "uat".to_string()),
            ],
        }
    }

    fn preview(&self, command: &Command) -> Option<Preview> {
        self.preview_for(command)
    }

    fn dispatch(&mut self, command: &Command) -> Outcome {
        match command {
            Command::Sync => self.sync_start(),
            Command::Claim(k) => self.claim(k),
            Command::StartReview(k) => self.start_review(k),
            Command::MarkReviewed(k) => self.mark_reviewed(k),
            Command::Activate { key, baseline } => self.activate(key, *baseline),
            Command::Park(k) => self.park(k),
            Command::ParkAndContinue { from, key, then } => {
                self.park_and_continue(from, key, *then)
            }
            Command::Recheck => {
                Outcome::ok().with_toast("Providers re-checked", ToastKind::Info, None)
            }
            Command::ToggleChecklist { key, index } => {
                self.toggle_checklist(key, *index);
                Outcome::ok()
            }
            Command::AddChecklist { key, text } => {
                self.add_checklist(key, text);
                Outcome::ok()
            }
            Command::SetNotes { key, text } => {
                if let Some(i) = self.idx(key) {
                    self.tickets[i].notes = text.clone();
                }
                Outcome::ok()
            }
            Command::Choose { key, repo, branch } => {
                self.choose(key, repo, branch);
                Outcome::ok()
            }
            Command::Prepare(k) => self.prepare(k),
            Command::TogglePrePush { key, repo, index } => {
                if let Some(i) = self.idx(key) {
                    let id = format!("{repo}:{index}");
                    if !self.tickets[i].pre.remove(&id) {
                        self.tickets[i].pre.insert(id);
                    }
                }
                Outcome::ok()
            }
            Command::PushUat(k) => self.push(k),
            Command::Rerun { key, repo } => self.rerun(key, repo),
            Command::ComposeDraft(k) => self.compose_draft(k),
            Command::EditDraft { key, id, text } => {
                self.edit_draft(key, id, text);
                Outcome::ok()
            }
            Command::PostAndMove { key, draft } => self.post_and_move(key, draft),
            Command::Transition(k) => self.transition(k),
            Command::Approve { key, repo, pr } => self.approve(key, repo, *pr),
            Command::ApproveAll(k) => self.approve_all(k),
            Command::RequestChanges { key, pr, text } => self.request_changes(key, *pr, text),
            Command::InlineComment {
                key,
                pr,
                file,
                line,
                text,
            } => self.add_thread(key, *pr, file, line, text),
            Command::PostComment { key, text } => self.post_free_comment(key, text),
            Command::ReturnMissingPr(k) => self.return_missing_pr(k),
            Command::ReturnThreads(k) => self.return_threads(k),
            Command::AcceptThreads(k) => self.accept_threads(k),
            Command::ConflictComment { key, also_return } => self.send_conflict(key, *also_return),
            Command::BreakLock(r) => self.break_lock(r),
            Command::ExtendWait { key, kind } => self.extend_wait(key, *kind),
            Command::Reclaim(k) => self.reclaim(k),
            Command::Dismiss(id) => self.dismiss(id),
            Command::Snooze { id, minutes } => self.snooze(id, *minutes),
            Command::Undo(u) => {
                match u {
                    Undo::Claim(k) => self.unclaim(k),
                    Undo::Reviewed {
                        key,
                        prev_seq,
                        prev_status,
                    } => {
                        self.unmark_reviewed(key, *prev_seq, prev_status);
                    }
                    Undo::Response(id) => {
                        self.responses.remove(id);
                    }
                }
                Outcome::ok()
            }
            Command::MarkSeen(k) => {
                self.mark_seen(k);
                Outcome::ok()
            }
            Command::SetProvider { provider, ready } => {
                match provider {
                    Provider::Jira => self.jira_ready = *ready,
                    Provider::GitHub => self.gh_ready = *ready,
                }
                Outcome::ok()
            }
            Command::SetWaitMinutes(m) => {
                self.wait_min = *m;
                Outcome::ok()
            }
            Command::StartWorkspace => {
                self.ws.up = true;
                self.ok("workspace.start", None, None, "");
                Outcome::ok().with_toast("Workspace started", ToastKind::Ok, None)
            }
            Command::StopWorkspace => {
                self.ws.up = false;
                self.ok("workspace.stop", None, None, "");
                Outcome::ok().with_toast("Workspace stopped", ToastKind::Info, None)
            }
            Command::ResetDemo => {
                *self = Sim::new();
                Outcome::ok().with_toast("Demo reset", ToastKind::Info, None)
            }
            Command::Sim(ev) => self.simulate(ev),
        }
    }

    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)> {
        self.advance(millis);
        std::mem::take(&mut self.toasts)
    }
}

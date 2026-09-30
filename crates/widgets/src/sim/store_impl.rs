//! `Store` for the simulation: builds every view model and routes every command.

use super::Sim;
use super::detail::{jira_badge, local_badge, priority_tone};
use super::model::*;
use super::rules::Sug;
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
        repo: a.repo.as_ref().map_or(String::new(), |r| r.to_string()),
        ticket: a.ticket.clone(),
        outcome: Badge::new(
            a.outcome.word(),
            match a.outcome {
                AuditOutcome::Success => Tone::Ok,
                AuditOutcome::Failure => Tone::Bad,
                AuditOutcome::Skipped => Tone::Neutral,
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
            attention: !s.info && (s.rule.needs_attention() || s.hotfix),
            rank: s.rank,
            hotfix: s.hotfix,
            info: s.info,
            ticket: s.ticket.clone(),
            open: s
                .ticket
                .clone()
                .zip(s.rule.tab())
                .map(|(k, tab)| Intent::go_ticket(k, tab)),
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
                        format!("{m} minutes"),
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
        v.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.key.cmp(&b.key)));
        v
    }

    fn ticket_row(&self, t: &Ticket) -> TicketRowVm {
        let un = Self::unseen_count(t);
        let mut flags = Vec::new();
        if !t.landings().is_empty() {
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
            jira: jira_badge(t.jira),
            local: local_badge(t.local()),
            repos: {
                let mut r: Vec<RepoName> = t.prs.iter().map(|p| p.repo.clone()).collect();
                r.sort();
                r.dedup();
                r
            },
            flags,
            urgent: matches!(t.priority, Priority::High | Priority::Highest),
            new_comments: un > 0,
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
                failed: a.outcome == AuditOutcome::Failure,
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

    /// The right panel on Home: what you are in the middle of, then what is waiting.
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
                    RightRow::Button(Btn::new(
                        "Open",
                        Intent::go_ticket(t.key.clone(), TicketTab::Test),
                    )),
                ],
            });
        }
        let queue: Vec<RightRow> = self
            .sorted_group(Group::Pool)
            .into_iter()
            .take(4)
            .map(|t| RightRow::Item {
                head: Vec::new(),
                key: Some(t.key.clone()),
                title: t.title.clone(),
                sub: Some(if self.is_hotfix(t) {
                    format!("{} · hotfix", t.priority.label())
                } else {
                    t.priority.label().to_string()
                }),
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
                title: "Time today".to_string(),
                rows,
            });
        }
        out.extend(self.sync_section());
        out.push(RightSection {
            title: "Recent activity".to_string(),
            rows: self.activity(None, 6),
        });
        out
    }

    /// The right panel on a ticket: who and when, the tags, the pull requests. What the header and the stepper
    /// already say (Jira status, local status, kind, reviewed) is not repeated here.
    fn right_ticket(&self, t: &Ticket) -> Vec<RightSection> {
        let tags = |v: &[&str]| -> Vec<Badge> {
            v.iter().map(|x| Badge::new(*x, Tone::Neutral)).collect()
        };
        let mut out = vec![RightSection {
            title: "People".to_string(),
            rows: vec![
                RightRow::Kv(Kv::text("Assignee", t.assignee)),
                RightRow::Kv(Kv::text("Reporter", t.reporter)),
            ],
        }];
        let mut plan = vec![
            RightRow::Kv(Kv::text("Type", t.kind)),
            RightRow::Kv(Kv::badges(
                "Priority",
                vec![Badge::new(t.priority.label(), priority_tone(t.priority))],
            )),
            RightRow::Kv(Kv::text("Sprint", t.sprint)),
            RightRow::Kv(Kv::text("Epic", t.epic)),
            RightRow::Kv(Kv::text("Fix version", t.fix_version)),
            RightRow::Kv(Kv::text("Estimate", t.estimate)),
        ];
        if self.duration_ms(t) > 0 {
            plan.push(RightRow::Kv(Kv::text(
                "Time spent",
                Self::fmt_dur(self.duration_ms(t)),
            )));
        }
        plan.push(RightRow::Muted(format!(
            "Created {} · updated {}",
            t.created, t.updated
        )));
        out.push(RightSection {
            title: "Planning".to_string(),
            rows: plan,
        });
        let mut all_tags = tags(&t.components);
        all_tags.extend(tags(&t.labels));
        if !all_tags.is_empty() {
            out.push(RightSection {
                title: "Tags".to_string(),
                rows: vec![RightRow::Tags(all_tags)],
            });
        }
        let prs: Vec<RightRow> = t
            .prs
            .iter()
            .map(|p| {
                let ap = p.reviewers.iter().filter(|r| r.approved).count();
                let mut sub = format!(
                    "{} #{} · {} → {} · {ap}/{} approved",
                    p.repo,
                    p.id,
                    p.src,
                    p.dst,
                    p.reviewers.len()
                );
                if let Some(l) = t.landing(&p.repo) {
                    sub.push_str(&format!(
                        " · run #{} {}",
                        l.deploy.run,
                        l.deploy.state.word()
                    ));
                }
                RightRow::Item {
                    head: Vec::new(),
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
                        head: Vec::new(),
                        key: self.tk(&l.key).map(|_| l.key.clone()),
                        title: l.title.to_string(),
                        sub: Some(format!("{} · {}", l.rel, l.status.label())),
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
            return Some(
                "Cannot send: acli is signed out. Run acli jira auth login, then re-check in Settings."
                    .to_string(),
            );
        }
        if gh && !self.gh_ready {
            return Some(
                "Cannot send: gh is signed out. Run gh auth login, then re-check in Settings."
                    .to_string(),
            );
        }
        None
    }

    /// A preview shell for `c`, with the signed-out check already applied.
    fn pv(
        &self,
        c: &Command,
        title: &str,
        ticket: Option<&TicketKey>,
        risk: Risk,
        summary: &str,
        confirm: &str,
    ) -> Preview {
        let mut p = Preview::new(c.clone(), title, risk, summary, confirm);
        p.ticket = ticket.cloned();
        p.blocked = self.blocked_for(c);
        p
    }

    /// The preview of every remote write. The match is over the flat command; `is_remote` and the test
    /// `every_remote_command_has_a_preview` keep the two in step.
    fn remote_preview(&self, rc: &RemoteCommand) -> Result<Preview, String> {
        let c = rc.command();
        let gone = || "That ticket is no longer in the cache. Nothing was sent.".to_string();
        match c {
            Command::PushUat(key) => {
                let t = self.tk(key).ok_or_else(gone)?;
                let prep = t
                    .prep()
                    .ok_or("Prepare the integration first. Nothing was sent.")?;
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
                let mut p = self.pv(
                    c,
                    "Push to uat",
                    Some(key),
                    Risk::High,
                    "Pushing deploys to alpha. No force: a rejected push stops and tells you.",
                    "Push to uat",
                );
                p.payload = payload;
                p.facts = facts;
                p.type_key = Some(key.to_string());
                Ok(p)
            }
            Command::Rerun { key, repo } => {
                let t = self.tk(key).ok_or_else(gone)?;
                let l = t.landing(repo).ok_or("There is no run to re-run.")?;
                let mut p = self.pv(
                    c,
                    "Re-run failed workflow",
                    Some(key),
                    Risk::Medium,
                    "Starts a new Actions run for the same commit. The failed run stays in the history.",
                    "Re-run",
                );
                p.payload = vec![format!(
                    "gh run rerun {} --repo {}",
                    l.deploy.run,
                    self.repo(repo).host
                )];
                Ok(p)
            }
            Command::PostAndMove { key, draft } => {
                let t = self.tk(key).ok_or_else(gone)?;
                if t.local() == Some(Local::Active) {
                    return Err(super::detail::REMERGE_FIRST.into());
                }
                let d = t
                    .drafts()
                    .iter()
                    .find(|d| d.id == *draft)
                    .ok_or("That draft is gone.")?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(d.body.lines().map(|l| format!("  {l}")));
                payload.push(format!(
                    "Transition: {} → {}",
                    JiraStatus::InReview.label(),
                    JiraStatus::AlphaTesting.label()
                ));
                let mut p = self.pv(
                    c,
                    &format!(
                        "Post the deploy comment and move to {}",
                        JiraStatus::AlphaTesting.label()
                    ),
                    Some(key),
                    Risk::Medium,
                    "One confirm sends the comment and moves the ticket.",
                    "Post and move",
                );
                p.payload = payload;
                Ok(p)
            }
            Command::Transition(key) => {
                let t = self.tk(key).ok_or_else(gone)?;
                if t.local() == Some(Local::Active) {
                    return Err(super::detail::REMERGE_FIRST.into());
                }
                let mut p = self.pv(
                    c,
                    &format!("Move to {}", JiraStatus::AlphaTesting.label()),
                    Some(key),
                    Risk::Medium,
                    "Changes the Jira status. The comment was already posted.",
                    "Move ticket",
                );
                p.payload = vec![format!(
                    "Transition: {} → {}",
                    JiraStatus::InReview.label(),
                    JiraStatus::AlphaTesting.label()
                )];
                Ok(p)
            }
            Command::Approve { key, repo, pr } => {
                self.tk(key).ok_or_else(gone)?;
                let mut p = self.pv(
                    c,
                    &format!("Approve {} #{pr}", self.repo(repo).host),
                    Some(key),
                    Risk::Medium,
                    "Approval is the last gate. It cannot be withdrawn from here.",
                    "Approve",
                );
                p.payload = vec![format!(
                    "gh pr review {pr} --approve --repo {}",
                    self.repo(repo).host
                )];
                Ok(p)
            }
            Command::ApproveAll(key) => {
                let t = self.tk(key).ok_or_else(gone)?;
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
                let mut p = self.pv(
                    c,
                    "Approve all pull requests",
                    Some(key),
                    Risk::Medium,
                    "Approval is the last gate. It cannot be withdrawn from here.",
                    "Approve all",
                );
                p.payload = payload;
                Ok(p)
            }
            Command::RequestChanges { key, pr, text } => {
                let mut p = self.pv(
                    c,
                    "Request changes",
                    Some(key),
                    Risk::Medium,
                    "Posts a review that blocks the merge until it is resolved.",
                    "Request changes",
                );
                p.payload = vec![format!("PR #{pr}"), format!("Body (verbatim): {text}")];
                Ok(p)
            }
            Command::InlineComment {
                key,
                pr,
                file,
                line,
                text,
            } => {
                let mut p = self.pv(
                    c,
                    "Comment on a line",
                    Some(key),
                    Risk::Medium,
                    "Posts an inline comment. If GitHub cannot anchor it, nothing is posted instead of a general comment.",
                    "Post comment",
                );
                p.payload = vec![
                    format!("PR #{pr} · {file} · {line}"),
                    format!("Body (verbatim): {text}"),
                ];
                Ok(p)
            }
            Command::PostComment { key, text } => {
                let mut p = self.pv(
                    c,
                    "Post a comment to Jira",
                    Some(key),
                    Risk::Low,
                    "Adds a comment to the ticket.",
                    "Post comment",
                );
                p.payload = vec!["Body (verbatim):".to_string(), format!("  {text}")];
                Ok(p)
            }
            Command::ReturnMissingPr(key) => {
                let t = self.tk(key).ok_or_else(gone)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(self.return_body(t).lines().map(|l| format!("  {l}")));
                payload.push(format!(
                    "Transition: {} → {}",
                    JiraStatus::InReview.label(),
                    JiraStatus::Returned.label()
                ));
                let mut p = self.pv(
                    c,
                    &format!("Return {key}: no pull request"),
                    Some(key),
                    Risk::Medium,
                    "Comments on the ticket and sends it back to development.",
                    "Return ticket",
                );
                p.payload = payload;
                Ok(p)
            }
            Command::ReturnThreads(key) => {
                let t = self.tk(key).ok_or_else(gone)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(
                    self.return_threads_body(t)
                        .lines()
                        .map(|l| format!("  {l}")),
                );
                payload.push(format!(
                    "Transition: {} → {}",
                    JiraStatus::InReview.label(),
                    JiraStatus::Returned.label()
                ));
                let mut p = self.pv(
                    c,
                    &format!("Return {key}: unresolved review comments"),
                    Some(key),
                    Risk::Medium,
                    "Comments on the ticket and sends it back to development.",
                    "Return ticket",
                );
                p.payload = payload;
                Ok(p)
            }
            Command::ConflictComment { key, .. } => {
                let t = self.tk(key).ok_or_else(gone)?;
                let mut payload = vec!["Comment body (verbatim):".to_string()];
                payload.extend(self.conflict_body(t).lines().map(|l| format!("  {l}")));
                let mut p = self.pv(
                    c,
                    &format!("Tell the developer {key} conflicts with uat"),
                    Some(key),
                    Risk::Medium,
                    "Adds a comment to the ticket. The ticket stays where it is.",
                    "Post comment",
                );
                p.payload = payload;
                Ok(p)
            }
            other => Err(format!("{other:?} is not a remote command")),
        }
    }

    /// Local commands that still ask first.
    fn guard_for(&self, lc: &LocalCommand) -> Option<Preview> {
        let c = lc.command();
        match c {
            Command::AcceptThreads(key) => {
                let t = self.tk(key)?;
                let mut p = self.pv(
                    c,
                    "Proceed with unresolved comments",
                    Some(key),
                    Risk::Low,
                    "Local only. Nothing is sent. The deploy comment will list these comments.",
                    "Proceed anyway",
                );
                p.payload = Self::blocking_threads(t)
                    .iter()
                    .map(Self::thread_line)
                    .collect();
                Some(p)
            }
            Command::Park(key) => {
                let t = self.tk(key)?;
                let mut p = self.pv(
                    c,
                    &format!("Park {key}"),
                    Some(key),
                    Risk::Low,
                    "Local only. Repos go back to where they were, stashes are restored and the overlay is reverted.",
                    "Park",
                );
                p.payload = t
                    .act()
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
                    .unwrap_or_default();
                Some(p)
            }
            Command::BreakLock(repo) => {
                let mut p = self.pv(
                    c,
                    &format!("Remove the stale lock in {repo}"),
                    None,
                    Risk::Low,
                    "Local only. Checks that no git process runs first.",
                    "Remove lock",
                );
                p.payload = vec![format!("rm {repo}/.git/index.lock")];
                Some(p)
            }
            Command::StopWorkspace => {
                let mut p = self.pv(
                    c,
                    "Stop the workspace",
                    None,
                    Risk::Medium,
                    "Stops every service. Uncommitted work stays on disk.",
                    "Stop",
                );
                p.payload =
                    vec!["docker compose stop (dependency order: web → api → db)".to_string()];
                p.facts = self
                    .ws
                    .dirty
                    .iter()
                    .map(|r| format!("{r} has uncommitted changes"))
                    .collect();
                Some(p)
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
                .and_then(|t| t.act())
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

    fn tab_info(&self, key: &TicketKey) -> Option<TabInfo> {
        let t = self.tk(key)?;
        Some(TabInfo {
            title: t.title.clone(),
            bad: !Self::failed_repos(t).is_empty()
                || !self.uat_conflicts(t).is_empty()
                || self.pr_wait_expired(t)
                || self.th_wait_expired(t),
            unseen: Self::unseen_count(t) > 0,
            active: t.local() == Some(Local::Active),
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
                label: "Home".into(),
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
        let total = sections.iter().map(|s| s.rows.len()).sum();
        let elsewhere = Group::NAV
            .iter()
            .filter(|g| **g != group)
            .map(|g| {
                (
                    *g,
                    self.tickets
                        .iter()
                        .filter(|t| Self::in_group(t, *g))
                        .count(),
                )
            })
            .find(|(_, n)| *n > 0);
        TicketListVm {
            show_local: group == Group::All,
            empty: if group == Group::All {
                "No tickets.".to_string()
            } else {
                format!("Nothing in {}.", group.label().to_lowercase())
            },
            sections,
            options: TicketFilterOptions::default(),
            filters: TicketFilters::default(),
            sort: None,
            total,
            elsewhere,
        }
    }

    fn ticket_exists(&self, key: &TicketKey) -> bool {
        self.tk(key).is_some()
    }

    fn ticket_head(&self, key: &TicketKey, _tab: TicketTab) -> Option<TicketHeadVm> {
        self.vm_head(key)
    }

    fn overview(&self, key: &TicketKey, seen: Option<u32>) -> Option<OverviewVm> {
        self.vm_overview(key, seen)
    }

    fn review(&self, key: &TicketKey, sel: &ReviewSel) -> Option<ReviewVm> {
        self.vm_review(key, sel)
    }

    fn test(&self, key: &TicketKey) -> Option<TestVm> {
        self.vm_test(key)
    }

    fn ship(&self, key: &TicketKey) -> Option<ShipVm> {
        self.vm_ship(key)
    }

    fn timeline(&self, key: &TicketKey) -> Vec<AuditRow> {
        self.audit
            .iter()
            .filter(|a| a.ticket.as_deref() == Some(key))
            .map(audit_row)
            .collect()
    }

    fn on_uat(&self) -> OnUatVm {
        let mut overlaps = Vec::new();
        for r in &self.repos {
            let list: Vec<&Ticket> = self
                .tickets
                .iter()
                .filter(|t| t.has_landed(&r.name))
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
        }
        let mut rows: Vec<TicketRowVm> = Vec::new();
        for t in self
            .tickets
            .iter()
            .filter(|t| self.repos.iter().any(|r| t.has_landed(&r.name)))
        {
            let mut row = self.ticket_row(t);
            // On uat a ticket's repos are the ones it landed in, not every repo it has a PR in.
            row.repos = self
                .repos
                .iter()
                .filter(|r| t.has_landed(&r.name))
                .map(|r| r.name.clone())
                .collect();
            row.repos.sort();
            rows.push(row);
        }
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        let total = rows.len();
        OnUatVm {
            table: TicketListVm {
                show_local: false,
                empty: "Nothing of yours is on uat.".to_string(),
                sections: if rows.is_empty() {
                    Vec::new()
                } else {
                    vec![TicketSection {
                        heading: None,
                        rows,
                    }]
                },
                options: TicketFilterOptions::default(),
                filters: TicketFilters::default(),
                sort: None,
                total,
                elsewhere: None,
            },
            overlaps,
        }
    }

    fn workspace(&self) -> WorkspaceVm {
        let active = self.active_key().and_then(|k| self.tk(&k));
        let rows = self
            .repos
            .iter()
            .map(|r| {
                // Only what is out of the ordinary gets a badge: the defaults (up, clean) are silence.
                let mut badges = Vec::new();
                if !self.ws.up {
                    badges.push(Badge::new("down", Tone::Neutral));
                }
                if self.ws.dirty.contains(&r.name) {
                    badges.push(Badge::new("uncommitted changes", Tone::Warn));
                }
                if active
                    .and_then(|t| t.act())
                    .and_then(|a| a.overlay.as_ref())
                    .is_some_and(|o| o.repo == r.name)
                {
                    badges.push(Badge::new("overlay", Tone::Neutral));
                }
                let mut break_lock = None;
                if let Some(l) = self.locks.get(&r.name) {
                    match l.kind {
                        LockKind::Stale => {
                            badges.push(Badge::new("stale index.lock", Tone::Bad));
                            break_lock = Some(Btn::new(
                                "Remove lock…",
                                cmd(Command::BreakLock(r.name.clone())),
                            ));
                        }
                        _ => badges.push(Badge::new(
                            format!("busy: {}", Self::lock_text(l)),
                            Tone::Warn,
                        )),
                    }
                }
                WsRow {
                    repo: r.name.clone(),
                    services: r.services,
                    branch: self.ws.branches.get(&r.name).cloned().unwrap_or_default(),
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
            appearance: Vec::new(),
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
                ("Review status".into(), JiraStatus::InReview.label().into()),
                (
                    "Alpha status".into(),
                    JiraStatus::AlphaTesting.label().into(),
                ),
                ("Returned".into(), JiraStatus::Returned.label().into()),
                ("Signed off".into(), JiraStatus::Done.label().into()),
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
                            .as_ref()
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
        let web = Some(RepoName::from("web"));
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

    fn comments_seen(&self, key: &TicketKey) -> u32 {
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

    fn baseline_choices(&self, key: &TicketKey) -> BaselineVm {
        let prod: Vec<&str> = {
            let mut v: Vec<&str> = self.repos.iter().map(|r| r.prod.as_str()).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        BaselineVm {
            key: key.clone(),
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

    fn preview(&self, command: &RemoteCommand) -> Result<Preview, String> {
        self.remote_preview(command)
    }

    fn local_guard(&self, command: &LocalCommand) -> Option<Preview> {
        self.guard_for(command)
    }

    fn dispatch(&mut self, command: LocalCommand) -> Outcome {
        self.run(command.command())
    }

    fn execute(&mut self, confirmed: Confirmed) -> Outcome {
        self.run(confirmed.command())
    }

    fn tick(&mut self, millis: i64) -> Vec<(String, ToastKind)> {
        self.advance(millis);
        std::mem::take(&mut self.toasts)
    }
}

impl Sim {
    /// Every command, local or confirmed remote, ends up here. The type-level split happens at the `Store` boundary.
    fn run(&mut self, command: &Command) -> Outcome {
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
                    let id = (repo.clone(), *index);
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
            } => self.add_thread(key, *pr, file, *line, text),
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
                        prev_mark,
                        prev_phase,
                    } => {
                        self.unmark_reviewed(key, *prev_mark, *prev_phase);
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
}

//! The next-action rules: pure over the world, ranked, each with a stated reason.
//! A simplified copy of the real engine in `de-core::next` (same rule names and bands, fewer inputs).

use super::Sim;
use super::model::*;
use crate::vm::{
    Badge, Command, Intent, Level, Route, SugState, SuggestionId, TicketKey, TicketTab, Tone,
};

#[derive(Clone, Debug)]
pub struct Sug {
    pub id: SuggestionId,
    pub ticket: Option<TicketKey>,
    pub rule: Rule,
    pub title: String,
    pub reason: String,
    /// What the row shows instead of the sentence when there are any: short facts, exceptions only.
    pub chips: Vec<Badge>,
    pub level: Level,
    pub info: bool,
    pub hotfix: bool,
    /// The tool a write needs: `acli` (Jira) or `gh` (GitHub).
    pub needs: Option<Needs>,
    pub act: Intent,
    pub alt: Option<(String, Intent)>,
    pub hash: String,
    /// The ticket's Jira priority and its place in the claim queue: inputs to the ranking (`ranking.rs`).
    pub ticket_priority: Option<Priority>,
    /// The ticket has no pull request yet and its grace period has not run out (or has not started): nothing
    /// can be done about it yet, so it must not lead the list.
    pub awaiting_pr: bool,
    pub queue: Option<usize>,
    pub rank: usize,
    pub state: SugState,
}

/// The rules of the next-action engine. Stable names; each has a priority band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rule {
    SyncStale,
    StaleLock,
    ThreadsReturn,
    ThreadsWaiting,
    PrMissingReturn,
    PrWaiting,
    UatConflict,
    UatMoved,
    PrepushChecks,
    DeployFailed,
    IntegrationBlocked,
    ReturnedToReview,
    ReturnedMention,
    ReReview,
    StartReview,
    FinishReview,
    ApprovePrs,
    IntegrateReady,
    PushReady,
    ComposeDeployComment,
    PostDeployComment,
    TransitionAlpha,
    RemergeNeeded,
    ActivateReviewed,
    ParkActive,
    DeployWaiting,
    ClaimNew,
    AdapterUnavailable,
}

impl Rule {
    /// The ticket tab where this suggestion's subject is: clicking the row opens it there. `None` for rules that
    /// are not about one ticket (sync, locks, tools), whose rows are not clickable.
    pub fn tab(self) -> Option<TicketTab> {
        match self {
            Rule::SyncStale | Rule::StaleLock | Rule::AdapterUnavailable => None,
            Rule::ThreadsReturn
            | Rule::ThreadsWaiting
            | Rule::UatConflict
            | Rule::ReReview
            | Rule::StartReview
            | Rule::FinishReview => Some(TicketTab::Review),
            Rule::PrMissingReturn
            | Rule::PrWaiting
            | Rule::ReturnedToReview
            | Rule::ReturnedMention
            | Rule::ActivateReviewed
            | Rule::ClaimNew => Some(TicketTab::Overview),
            Rule::ParkActive => Some(TicketTab::Test),
            Rule::UatMoved
            | Rule::PrepushChecks
            | Rule::DeployFailed
            | Rule::IntegrationBlocked
            | Rule::ApprovePrs
            | Rule::IntegrateReady
            | Rule::PushReady
            | Rule::ComposeDeployComment
            | Rule::PostDeployComment
            | Rule::TransitionAlpha
            | Rule::RemergeNeeded
            | Rule::DeployWaiting => Some(TicketTab::Ship),
        }
    }

    pub(super) fn band(self) -> i32 {
        match self {
            Rule::StaleLock => 880,
            Rule::ThreadsReturn => 855,
            Rule::ThreadsWaiting => 545,
            Rule::PrMissingReturn => 860,
            Rule::PrWaiting => 550,
            Rule::UatConflict => 900,
            Rule::UatMoved => 700,
            Rule::PrepushChecks => 678,
            Rule::DeployFailed => 1000,
            Rule::IntegrationBlocked => 990,
            Rule::ReturnedToReview => 850,
            Rule::ReturnedMention => 840,
            Rule::ReReview => 830,
            Rule::StartReview => 820,
            Rule::FinishReview => 810,
            Rule::ApprovePrs => 690,
            Rule::IntegrateReady => 680,
            Rule::PushReady => 675,
            Rule::ComposeDeployComment => 670,
            Rule::PostDeployComment => 665,
            Rule::TransitionAlpha => 660,
            Rule::RemergeNeeded => 645,
            Rule::ActivateReviewed => 640,
            Rule::ParkActive => 620,
            Rule::DeployWaiting => 600,
            Rule::ClaimNew => 480,
            Rule::AdapterUnavailable => 299,
            Rule::SyncStale => 1200,
        }
    }

    /// Something is broken, someone is waiting, or a hotfix is in play.
    pub(crate) fn needs_attention(self) -> bool {
        matches!(
            self,
            Rule::StaleLock
                | Rule::ThreadsReturn
                | Rule::PrMissingReturn
                | Rule::UatConflict
                | Rule::DeployFailed
                | Rule::IntegrationBlocked
                | Rule::ReturnedMention
                | Rule::ReturnedToReview
                | Rule::ReReview
                | Rule::ApprovePrs
                | Rule::AdapterUnavailable
        )
    }
}

/// The tool a write needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// `acli` (Jira).
    Jira,
    /// `gh` (GitHub).
    GitHub,
}

impl Needs {
    fn tool(self) -> &'static str {
        match self {
            Needs::Jira => "acli",
            Needs::GitHub => "gh",
        }
    }
}

fn cmd(c: Command) -> Intent {
    Intent::Do(c)
}

fn open(key: &TicketKey, tab: TicketTab) -> Intent {
    Intent::Go(Route::ticket(key.clone(), tab))
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

impl Sim {
    /// The facts of a claim suggestion as chips: a hotfix and a missing pull request. The priority is the mark
    /// beside the key (as in the ticket table) and the place in the queue is the order of the rows.
    fn claim_chips(hot: bool, no_pr: bool) -> Vec<Badge> {
        let mut chips = Vec::new();
        if hot {
            chips.push(Badge::new("Hotfix", Tone::Hot));
        }
        if no_pr {
            chips.push(Badge::new("No pull request", Tone::Warn));
        }
        chips
    }

    pub(super) fn base_sug(
        rule: Rule,
        id: String,
        title: String,
        reason: String,
        act: Intent,
        hash: String,
    ) -> Sug {
        Sug {
            id: id.into(),
            ticket: None,
            rule,
            title,
            reason,
            chips: Vec::new(),
            level: Level::Local,
            info: false,
            hotfix: false,
            needs: None,
            act,
            alt: None,
            hash,
            ticket_priority: None,
            awaiting_pr: false,
            queue: None,
            rank: 0,
            state: SugState::Open,
        }
    }

    /// All suggestions, ranked, with their response state. Hidden ones are included.
    pub(crate) fn suggest(&self) -> Vec<Sug> {
        let mut out: Vec<Sug> = Vec::new();
        let act = self.active_key();
        let mut push = |mut s: Sug, queue: Option<usize>| {
            s.queue = queue;
            s.ticket_priority = s
                .ticket
                .as_ref()
                .and_then(|k| self.tk(k))
                .map(|t| t.priority);
            s.awaiting_pr = s
                .ticket
                .as_ref()
                .and_then(|k| self.tk(k))
                .is_some_and(|t| self.pr_gap(t).is_some() && !self.pr_wait_expired(t));
            out.push(s);
        };

        // sync
        let age = self.mins_ago(self.sync.last_ok);
        let since_attempt = self.mins_ago(self.sync.last_attempt);
        if !self.sync.running
            && ((self.sync.last_error.is_some() && since_attempt >= 15)
                || (self.sync.last_error.is_none() && age >= self.stale_sync_min && since_attempt >= 1))
        {
            let mut s = Self::base_sug(
                Rule::SyncStale,
                "-:sync_stale:all".to_string(),
                "Sync now".to_string(),
                format!("Data was last refreshed {age} min ago."),
                cmd(Command::Sync),
                "sync".to_string(),
            );
            s.level = Level::Automatic;
            push(s, None);
        }

        for (repo, l) in &self.locks {
            if matches!(l.kind, LockKind::Stale) {
                push(
                    Self::base_sug(
                        Rule::StaleLock,
                        format!("-:stale_lock:{repo}"),
                        format!("Stale git lock in {repo}"),
                        format!(
                            "{repo} has a leftover .git/index.lock and no git process is running, so nothing can change in that repo. Remove the lock."
                        ),
                        cmd(Command::BreakLock(repo.clone())),
                        format!("sl{repo}"),
                    ),
                    None,
                );
            }
        }

        for t in &self.tickets {
            let key = &t.key;
            let st = t.local();
            let hot = self.is_hotfix(t);
            let mk = |rule: Rule,
                      id: String,
                      title: String,
                      reason: String,
                      act: Intent,
                      hash: String| {
                let mut s = Self::base_sug(rule, id, title, reason, act, hash);
                s.ticket = Some(key.clone());
                s.hotfix = hot;
                s
            };

            // mentions
            let ms = Self::unseen_mentions(t);
            if !ms.is_empty()
                && matches!(
                    st,
                    None | Some(
                        Local::Integrated
                            | Local::Done
                            | Local::Claimed
                            | Local::Reviewing
                            | Local::Parked
                            | Local::Active
                    )
                )
            {
                let last = ms[ms.len() - 1];
                let mut s = mk(
                    Rule::ReturnedMention,
                    format!("{key}:returned_mention"),
                    if t.jira == JiraStatus::Returned {
                        "Returned: you were mentioned".to_string()
                    } else {
                        "You were mentioned".to_string()
                    },
                    format!(
                        "{} mentioned you on {key} ({} unseen comment{}).",
                        last.who,
                        ms.len(),
                        plural(ms.len())
                    ),
                    open(key, TicketTab::Overview),
                    format!("m{}", ms.len()),
                );
                s.info = true;
                push(s, None);
            }
            if t.jira == JiraStatus::InReview
                && matches!(st, Some(Local::Integrated | Local::Done))
                && !t.new_commits
                && Self::draft(t).is_some_and(|d| d.posted)
                && !t.landings().is_empty()
            {
                push(
                    mk(
                        Rule::ReturnedToReview,
                        format!("{key}:returned_to_review"),
                        "Back in the Review column".to_string(),
                        format!(
                            "{key} returned to Review after alpha. Treat it as new: claim it again."
                        ),
                        cmd(Command::Reclaim(key.clone())),
                        "rr".to_string(),
                    ),
                    None,
                );
            }

            if self.conflict_sent(t).is_none() {
                for (repo, with, files) in self.uat_conflicts(t) {
                    let mut s = mk(
                        Rule::UatConflict,
                        format!("{key}:uat_conflict:{repo}"),
                        format!("Conflict with uat in {repo}"),
                        format!(
                            "{key} will not merge into uat: it conflicts with {with} in {}. Send the developer a prepared comment, and return it if you like.",
                            files.join(", ")
                        ),
                        cmd(Command::ConflictComment {
                            key: key.clone(),
                            also_return: false,
                        }),
                        format!("uc{with}"),
                    );
                    s.level = Level::External;
                    s.needs = Some(Needs::Jira);
                    push(s, None);
                }
            }

            if st.is_some() && self.pr_gap(t).is_some() {
                let left = self.wait_left(t.pr_wait);
                if let (Some(w), Some(l)) = (t.pr_wait, left)
                    && l <= 0
                {
                    let mut s = mk(
                        Rule::PrMissingReturn,
                        format!("{key}:pr_return"),
                        format!("Return {key}: no pull request after {} min", w.mins),
                        format!(
                            "{} Review cannot start. Return it with a comment that explains.",
                            self.gap_text(t)
                        ),
                        cmd(Command::ReturnMissingPr(key.clone())),
                        format!("pr{}:{}", w.since, w.mins),
                    );
                    s.level = Level::External;
                    s.needs = Some(Needs::Jira);
                    push(s, None);
                } else {
                    let mins = left.map_or(0, |l| {
                        (l + super::model::MS_PER_MIN - 1) / super::model::MS_PER_MIN
                    });
                    let mut s = mk(
                        Rule::PrWaiting,
                        format!("{key}:pr_waiting"),
                        "Waiting for the pull request".to_string(),
                        format!(
                            "{} {mins} min left. It clears itself when the PR appears.",
                            self.gap_text(t)
                        ),
                        open(key, TicketTab::Overview),
                        "pw".to_string(),
                    );
                    s.info = true;
                    s.level = Level::Automatic;
                    push(s, None);
                }
                continue;
            }

            let bt = Self::blocking_threads(t);
            if !bt.is_empty() && t.reviewed() && st.is_some() {
                let left = self.wait_left(t.th_wait);
                let n = bt.len();
                if let (Some(w), Some(l)) = (t.th_wait, left)
                    && l <= 0
                {
                    let mut s = mk(Rule::ThreadsReturn,
                        format!("{key}:threads_return"),
                        format!("Return {key}: {n} unresolved comment{} after {} min", plural(n), w.mins),
                        "The review comments are still open, so activation is blocked. Return it with a comment that lists them, or proceed anyway.".to_string(),
                        cmd(Command::ReturnThreads(key.clone())),
                        format!("tr{}:{}:{n}", w.since, w.mins),
                    );
                    s.level = Level::External;
                    s.needs = Some(Needs::Jira);
                    s.alt = Some((
                        "Proceed anyway…".to_string(),
                        cmd(Command::AcceptThreads(key.clone())),
                    ));
                    push(s, None);
                } else {
                    let wait = left.map_or(String::new(), |l| {
                        format!("{} min left. ", (l + MS_PER_MIN - 1) / MS_PER_MIN)
                    });
                    let mut s = mk(
                        Rule::ThreadsWaiting,
                        format!("{key}:threads_waiting"),
                        format!("Waiting for {n} comment{} to be resolved", plural(n)),
                        format!(
                            "Activation is blocked until they are. {wait}It clears itself when they are resolved."
                        ),
                        open(key, TicketTab::Overview),
                        format!("tw{n}"),
                    );
                    s.info = true;
                    s.level = Level::Automatic;
                    s.alt = Some((
                        "Proceed anyway…".to_string(),
                        cmd(Command::AcceptThreads(key.clone())),
                    ));
                    push(s, None);
                }
                continue;
            }

            if st.is_none() && t.jira == JiraStatus::InReview {
                // unclaimed: handled by the claim queue below
            } else {
                if st == Some(Local::Claimed) {
                    push(
                        mk(
                            Rule::StartReview,
                            format!("{key}:start_review"),
                            "Start the review".to_string(),
                            format!(
                                "{key} is claimed. Read its {} PR{} against {}.",
                                t.prs.len(),
                                plural(t.prs.len()),
                                {
                                    let mut d: Vec<&str> =
                                        t.prs.iter().map(|p| p.dst.as_str()).collect();
                                    d.dedup();
                                    d.join(", ")
                                }
                            ),
                            cmd(Command::StartReview(key.clone())),
                            "s".to_string(),
                        ),
                        None,
                    );
                }
                if st == Some(Local::Reviewing) && !t.reviewed() {
                    push(
                        mk(Rule::FinishReview,
                            format!("{key}:finish_review"),
                            "Finish the review".to_string(),
                            "Review in progress. Mark it reviewed when the diffs and comments are done.".to_string(),
                            open(key, TicketTab::Review),
                            "f".to_string(),
                        ),
                        None,
                    );
                }
                if Self::stale_review(t)
                    && matches!(st, Some(Local::Reviewing | Local::Parked | Local::Claimed))
                {
                    let seq = t.prs.iter().map(|p| p.updated_seq).max().unwrap_or(0);
                    push(
                        mk(
                            Rule::ReReview,
                            format!("{key}:re_review"),
                            "Re-review: new commits".to_string(),
                            format!("The branches moved since you reviewed {key}."),
                            cmd(Command::StartReview(key.clone())),
                            format!("rv{seq}"),
                        ),
                        None,
                    );
                }

                if act.is_none()
                    && t.reviewed()
                    && matches!(st, Some(Local::Reviewing | Local::Parked))
                    && t.jira != JiraStatus::Returned
                {
                    push(
                        mk(
                            Rule::ActivateReviewed,
                            format!("{key}:activate"),
                            if hot {
                                "Activate hotfix (choose baseline)".to_string()
                            } else {
                                "Activate for local test".to_string()
                            },
                            format!(
                                "Reviewed{}. Nothing is active. This switches the touched repos to the ticket branches and stashes local changes.",
                                if st == Some(Local::Parked) {
                                    " and parked"
                                } else {
                                    ""
                                }
                            ),
                            cmd(Command::Activate {
                                key: key.clone(),
                                baseline: None,
                            }),
                            "a".to_string(),
                        ),
                        None,
                    );
                }
                if let Some(a) = &act
                    && a != key
                    && hot
                    && t.reviewed()
                    && matches!(st, Some(Local::Reviewing | Local::Parked))
                {
                    let mut s = Self::base_sug(
                        Rule::ParkActive,
                        format!("{a}:park_active:{key}"),
                        format!("Park {a} for the hotfix {key}"),
                        format!(
                            "{key} is a hotfix waiting to be tested. Parking restores the repos and reverts the overlay."
                        ),
                        cmd(Command::Park(a.clone())),
                        "pa".to_string(),
                    );
                    s.ticket = Some(a.clone());
                    s.hotfix = true;
                    push(s, None);
                }

                if st == Some(Local::Active) {
                    let done = !t.checklist.is_empty() && t.checklist.iter().all(|c| c.1);
                    let pushed = self.pushed_all(t);
                    let touched = self.touched(t).len();
                    let blocked = t.prep().as_ref().is_some_and(|p| {
                        p.rows.iter().any(|r| {
                            matches!(
                                r.outcome,
                                PrepOutcome::Conflict { .. } | PrepOutcome::Blocked { .. }
                            )
                        })
                    });
                    if done && t.prep().is_none() && !pushed {
                        push(
                            mk(
                                Rule::IntegrateReady,
                                format!("{key}:integrate_ready"),
                                "Prepare integration to uat".to_string(),
                                format!(
                                    "Checklist complete ({} of {}). {touched} repo{} touched. Preparing pushes nothing.",
                                    t.checklist.len(),
                                    t.checklist.len(),
                                    plural(touched)
                                ),
                                cmd(Command::Prepare(key.clone())),
                                "i".to_string(),
                            ),
                            None,
                        );
                    }
                    let missing = self.prepush_missing(t);
                    if Self::prep_ready(t) && !missing.is_empty() {
                        push(
                            mk(
                                Rule::PrepushChecks,
                                format!("{key}:prepush"),
                                "Tick the pre-push checks".to_string(),
                                format!(
                                    "{} check{} left before the push unlocks: {}.",
                                    missing.len(),
                                    plural(missing.len()),
                                    missing
                                        .iter()
                                        .map(|(r, x)| format!("{r}: {x}"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                                open(key, TicketTab::Ship),
                                format!("pp{}", missing.len()),
                            ),
                            None,
                        );
                    }
                    if let Some(prep) = &t.prep()
                        && self.prep_pushable(t)
                    {
                        let ready: Vec<&str> = prep
                            .rows
                            .iter()
                            .filter(|r| matches!(r.outcome, PrepOutcome::Ready { .. }))
                            .map(|r| r.repo.as_str())
                            .collect();
                        let mut s = mk(
                            Rule::PushReady,
                            format!("{key}:push_ready"),
                            "Review and push to uat".to_string(),
                            format!(
                                "Merge ready in {}. Pushing deploys to alpha.",
                                ready.join(", ")
                            ),
                            cmd(Command::PushUat(key.clone())),
                            format!("p{}", prep.at),
                        );
                        s.level = Level::External;
                        push(s, None);
                    }
                    if blocked && let Some(prep) = &t.prep() {
                        for r in &prep.rows {
                            let (title, reason, tag) = match &r.outcome {
                                PrepOutcome::Conflict { files, with } => (
                                    format!("Resolve the uat conflict in {}", r.repo),
                                    format!(
                                        "Merging into uat conflicts with {with} in {}. Nothing was pushed.",
                                        files.join(", ")
                                    ),
                                    "conflict",
                                ),
                                PrepOutcome::Blocked { reason } => (
                                    format!("Integration blocked in {}", r.repo),
                                    reason.clone(),
                                    "blocked",
                                ),
                                _ => continue,
                            };
                            let mut s = mk(
                                Rule::IntegrationBlocked,
                                format!("{key}:blocked:{}", r.repo),
                                title,
                                reason,
                                open(key, TicketTab::Ship),
                                format!("{tag}{}", r.repo),
                            );
                            s.info = true;
                            push(s, None);
                        }
                    }
                }

                let sent_back = t.jira == JiraStatus::Returned;
                let mut skip_rest = false;
                if !t.landings().is_empty() {
                    for repo in Self::failed_repos(t) {
                        let Some(l) = t.landing(&repo) else { continue };
                        let d = &l.deploy;
                        let mut s = mk(
                            Rule::DeployFailed,
                            format!("{key}:deploy_failed:{repo}"),
                            format!("Deploy failed in {repo}"),
                            format!(
                                "The deploy step of run #{} failed after your push to uat.",
                                d.run
                            ),
                            cmd(Command::Rerun {
                                key: key.clone(),
                                repo: repo.clone(),
                            }),
                            format!("df{}", d.run),
                        );
                        s.level = Level::External;
                        s.needs = Some(Needs::GitHub);
                        push(s, None);
                    }
                    if sent_back {
                        skip_rest = true;
                    } else {
                        let moved = t.landings().iter().find_map(|l| l.deploy.uat_moved.clone());
                        if let Some(u) = moved
                            && !t.jira.is_signed_off()
                        {
                            let mut s = mk(
                                Rule::UatMoved,
                                format!("{key}:uat_moved"),
                                "uat moved after your push".to_string(),
                                format!(
                                    "{} ({}) was pushed to uat at {}. Alpha now has both, so check the combined behaviour.",
                                    u.ticket, u.by, u.at
                                ),
                                open(key, TicketTab::Ship),
                                format!("um{}", u.at),
                            );
                            s.info = true;
                            push(s, None);
                        }
                        if Self::any_running(t) && Self::failed_repos(t).is_empty() {
                            let running: Vec<String> = t
                                .landings()
                                .iter()
                                .filter_map(|l| {
                                    let d = &l.deploy;
                                    matches!(d.state, DeployState::Pending | DeployState::Running)
                                        .then(|| format!("{} #{} ({})", l.repo, d.run, d.step))
                                })
                                .collect();
                            let mut s = mk(
                                Rule::DeployWaiting,
                                format!("{key}:deploy_waiting"),
                                "Waiting for Actions runs".to_string(),
                                format!("{}.", running.join(", ")),
                                open(key, TicketTab::Ship),
                                "w".to_string(),
                            );
                            s.info = true;
                            s.level = Level::Automatic;
                            push(s, None);
                        }
                        let d = Self::draft(t);
                        if self.all_deployed(t) && d.is_none() {
                            push(
                                mk(Rule::ComposeDeployComment,
                                    format!("{key}:compose"),
                                    "Draft the deploy comment".to_string(),
                                    "Every touched repo is deployed to alpha. The draft lists each repo with its PR and Actions run.".to_string(),
                                    cmd(Command::ComposeDraft(key.clone())),
                                    "c".to_string(),
                                ),
                                None,
                            );
                        }
                        if let Some(d) = d {
                            if !d.posted {
                                let mut s = mk(Rule::PostDeployComment,
                                    format!("{key}:post"),
                                    format!(
                                        "Post the deploy comment and move to {}",
                                        JiraStatus::AlphaTesting.label()
                                    ),
                                    "A draft is ready with the deploy, your checklist and the comments from testing. Edit it, then post; the ticket moves in the same step.".to_string(),
                                    cmd(Command::PostAndMove { key: key.clone(), draft: d.id.clone() }),
                                    format!("pc{}", d.id),
                                );
                                s.level = Level::External;
                                s.needs = Some(Needs::Jira);
                                push(s, None);
                            } else if t.jira == JiraStatus::InReview {
                                let mut s = mk(
                                    Rule::TransitionAlpha,
                                    format!("{key}:transition"),
                                    format!("Move to {}", JiraStatus::AlphaTesting.label()),
                                    format!(
                                        "The comment is posted but the ticket did not move. Transition {} → {}.",
                                        JiraStatus::InReview.label(),
                                        JiraStatus::AlphaTesting.label()
                                    ),
                                    cmd(Command::Transition(key.clone())),
                                    "t".to_string(),
                                );
                                s.level = Level::External;
                                s.needs = Some(Needs::Jira);
                                push(s, None);
                            }
                        }
                    }
                }
                if skip_rest {
                    // a returned ticket is only your business when mentioned or failing
                } else {
                    if st == Some(Local::Integrated) && t.new_commits && act.is_none() {
                        push(
                            mk(Rule::RemergeNeeded,
                                format!("{key}:remerge"),
                                if hot {
                                    "Re-merge needed (choose baseline)".to_string()
                                } else {
                                    "Re-merge needed".to_string()
                                },
                                "The ticket branches have commits that uat lacks after your recorded merge. Activate it again, test, and integrate again.".to_string(),
                                cmd(Command::Activate { key: key.clone(), baseline: None }),
                                "rm".to_string(),
                            ),
                            None,
                        );
                    }
                    if st == Some(Local::Integrated) && t.jira.is_signed_off() {
                        let pend: Vec<&Pr> = t
                            .prs
                            .iter()
                            .filter(|p| !p.reviewers.iter().any(|r| r.name == ME && r.approved))
                            .collect();
                        if pend.len() > 1 {
                            let mut s = mk(
                                Rule::ApprovePrs,
                                format!("{key}:approve:all"),
                                format!("Approve {} pull requests", pend.len()),
                                format!(
                                    "{key} passed testing (Jira: {}). {}. Approval is the last gate.",
                                    t.jira.label(),
                                    pend.iter()
                                        .map(|p| format!("{} #{}", p.repo, p.id))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                                cmd(Command::ApproveAll(key.clone())),
                                format!("apa{}", pend.len()),
                            );
                            s.level = Level::External;
                            s.needs = Some(Needs::GitHub);
                            push(s, None);
                        } else {
                            for p in pend {
                                let mut s = mk(
                                    Rule::ApprovePrs,
                                    format!("{key}:approve:{}#{}", p.repo, p.id),
                                    format!("Approve {} #{}", self.repo(&p.repo).host, p.id),
                                    format!(
                                        "{key} passed testing (Jira: {}). Approval is the last gate.",
                                        t.jira.label()
                                    ),
                                    cmd(Command::Approve {
                                        key: key.clone(),
                                        repo: p.repo.clone(),
                                        pr: p.id,
                                    }),
                                    format!("ap{}", p.id),
                                );
                                s.level = Level::External;
                                s.needs = Some(Needs::GitHub);
                                push(s, None);
                            }
                        }
                    }
                }
            }
        }

        // claim queue
        let mut pool: Vec<&Ticket> = self
            .tickets
            .iter()
            .filter(|t| t.local().is_none() && t.jira == JiraStatus::InReview)
            .collect();
        pool.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.key.cmp(&b.key)));
        let eligible: Vec<&&Ticket> = pool
            .iter()
            .filter(|t| self.in_hand(&t.key).is_none() || self.is_hotfix(t))
            .collect();
        for (i, t) in eligible.iter().enumerate() {
            let hot = self.is_hotfix(t);
            let mut s = Self::base_sug(
                Rule::ClaimNew,
                format!("{}:claim_new", t.key),
                format!("Claim {}", t.key),
                format!(
                    "In the Review column, priority {}{}{}.",
                    t.priority.label(),
                    if hot {
                        ", targets production (hotfix)"
                    } else {
                        ""
                    },
                    if self.pr_gap(t).is_some() {
                        ", but it has no pull request yet"
                    } else {
                        ""
                    }
                ),
                cmd(Command::Claim(t.key.clone())),
                "q".to_string(),
            );
            s.ticket = Some(t.key.clone());
            s.hotfix = hot;
            s.chips = Self::claim_chips(hot, self.pr_gap(t).is_some());
            push(s, Some(i));
        }

        // A write whose tool is signed out becomes a notice about the tool instead.
        let mut res: Vec<Sug> = Vec::new();
        for s in out {
            if let Some(n) = s.needs {
                let ready = match n {
                    Needs::Jira => self.jira_ready,
                    Needs::GitHub => self.gh_ready,
                };
                if !ready {
                    res.push(Sug {
                        id: SuggestionId::new(format!("-:adapter:{}:{:?}", n.tool(), s.rule)),
                        ticket: s.ticket.clone(),
                        rule: Rule::AdapterUnavailable,
                        title: format!("{} is signed out", n.tool()),
                        reason: format!(
                            "Cannot {} until {} is logged in. Run {} auth login.",
                            s.title.to_lowercase(),
                            n.tool(),
                            n.tool()
                        ),
                        chips: Vec::new(),
                        level: Level::Automatic,
                        info: true,
                        hotfix: s.hotfix,
                        needs: None,
                        act: Intent::Go(Route::Settings),
                        alt: None,
                        hash: "ad".to_string(),
                        ticket_priority: s.ticket_priority,
                        awaiting_pr: s.awaiting_pr,
                        queue: None,
                        rank: 0,
                        state: SugState::Open,
                    });
                    continue;
                }
            }
            res.push(s);
        }
        let mut seen = std::collections::BTreeSet::new();
        res.retain(|s| seen.insert(s.id.clone()));
        res.sort_by(super::ranking::compare);
        for (i, s) in res.iter_mut().enumerate() {
            s.rank = i + 1;
            s.state = self.sug_state(s);
        }
        res
    }

    fn sug_state(&self, s: &Sug) -> SugState {
        match self.responses.get(&s.id) {
            None => SugState::Open,
            Some(Response::Dismissed { hash }) => {
                if *hash == s.hash {
                    SugState::Dismissed
                } else {
                    SugState::Resurfaced
                }
            }
            Some(Response::Snoozed { until }) => {
                if self.ms < *until {
                    SugState::Snoozed
                } else {
                    SugState::Open
                }
            }
        }
    }

    pub(crate) fn visible_suggestions(&self) -> Vec<Sug> {
        self.suggest()
            .into_iter()
            .filter(|s| matches!(s.state, SugState::Open | SugState::Resurfaced))
            .collect()
    }

    /// What needs you now.
    pub(crate) fn attention(&self) -> Vec<Sug> {
        let mut v: Vec<Sug> = self
            .visible_suggestions()
            .into_iter()
            .filter(|s| !s.info && (s.rule.needs_attention() || s.hotfix))
            .collect();
        v.sort_by(super::ranking::compare);
        v
    }
}

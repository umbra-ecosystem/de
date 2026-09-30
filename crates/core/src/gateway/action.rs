//! The typed external actions, their previews, and the draft -> confirm chain.
//!
//! `Draft::confirm` is the only way to obtain a [`Confirmed`], and `Gateway::execute`
//! accepts nothing else. A `Confirmed` carries a hash of the payload that was previewed;
//! execution recomputes it, so a payload changed after the preview is rejected.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    domain::TicketKey,
    providers::{NewPrComment, TriggerSpec},
};

/// How much damage a wrong confirmation can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
    /// Deploys, or cannot be undone.
    High,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

/// One commit shown in a push preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitLine {
    pub sha: String,
    pub summary: String,
}

/// One repo of a `uat` push. Every field is part of the confirmed payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushUatRepo {
    /// Workspace project name.
    pub repo: String,
    /// The user's checkout (the push runs here; it shares objects with the temp worktree).
    pub repo_dir: PathBuf,
    pub remote: String,
    pub uat_branch: String,
    pub ticket_branch: String,
    /// SHA of the ticket branch tip that was merged.
    pub ticket_tip: String,
    /// SHA of `uat` the merge was made on.
    pub uat_before: String,
    /// SHA of the merge commit to push.
    pub merge_commit: String,
    /// Ticket commits being merged.
    pub commits: Vec<CommitLine>,
    /// Files changed by the merge relative to `uat`.
    pub files: Vec<String>,
    /// Composer packages of the repo's overlay config, for the guard re-run at push time.
    pub overlay_packages: Vec<String>,
}

/// Push the merge commits of a ticket to `uat`, repo by repo in order. Deploys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushUat {
    pub ticket: TicketKey,
    pub repos: Vec<PushUatRepo>,
}

/// Everything the app may write to an external system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    PushUat(PushUat),
    PostJiraComment {
        ticket: TicketKey,
        body: String,
    },
    TransitionJira {
        ticket: TicketKey,
        to_status: String,
    },
    PostPrComment {
        repo: String,
        pr: u64,
        comment: NewPrComment,
    },
    ApprovePr {
        repo: String,
        pr: u64,
    },
    RequestChanges {
        repo: String,
        pr: u64,
        body: String,
    },
    TriggerPipeline {
        repo: String,
        spec: TriggerSpec,
    },
    RerunPipeline {
        repo: String,
        run_id: String,
    },
}

/// A titled group of preview lines (per repo for a push).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewSection {
    pub heading: String,
    pub lines: Vec<String>,
}

/// What the human is shown before confirming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionPreview {
    pub title: String,
    pub risk: Risk,
    /// The literal payload that will be sent (the comment body, the status name, the git
    /// commands), exactly as it will go out.
    pub payload: String,
    pub sections: Vec<PreviewSection>,
}

impl ActionPreview {
    /// Plain lines for a terminal.
    pub fn to_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("{}  [risk: {}]", self.title, self.risk.as_str()),
            String::new(),
        ];
        for section in &self.sections {
            lines.push(section.heading.clone());
            lines.extend(section.lines.iter().map(|l| format!("  {l}")));
            lines.push(String::new());
        }
        lines.push("Exactly what will be sent:".into());
        lines.extend(self.payload.lines().map(|l| format!("  | {l}")));
        lines
    }
}

/// The ticket an action concerns, when it names one.
impl Action {
    pub fn ticket(&self) -> Option<&TicketKey> {
        match self {
            Action::PushUat(p) => Some(&p.ticket),
            Action::PostJiraComment { ticket, .. } | Action::TransitionJira { ticket, .. } => {
                Some(ticket)
            }
            _ => None,
        }
    }

    /// The audit action name of this kind of action.
    pub fn audit_name(&self) -> &'static str {
        use super::actions::*;
        match self {
            Action::PushUat(_) => PUSH_UAT,
            Action::PostJiraComment { .. } => JIRA_COMMENT,
            Action::TransitionJira { .. } => JIRA_TRANSITION,
            Action::PostPrComment { .. } => PR_COMMENT,
            Action::ApprovePr { .. } => PR_APPROVE,
            Action::RequestChanges { .. } => PR_REQUEST_CHANGES,
            Action::TriggerPipeline { .. } => PIPELINE_TRIGGER,
            Action::RerunPipeline { .. } => PIPELINE_RERUN,
        }
    }

    pub fn risk(&self) -> Risk {
        match self {
            Action::PushUat(_) => Risk::High,
            _ => Risk::Medium,
        }
    }

    /// The canonical payload the hash covers.
    pub(crate) fn canonical(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub(crate) fn hash(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.canonical().hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Renders the exact preview of this action.
    pub fn preview(&self) -> ActionPreview {
        match self {
            Action::PushUat(push) => preview_push(push),
            Action::PostJiraComment { ticket, body } => ActionPreview {
                title: format!("Post a comment on {ticket}"),
                risk: self.risk(),
                payload: body.clone(),
                sections: vec![],
            },
            Action::TransitionJira { ticket, to_status } => ActionPreview {
                title: format!("Move {ticket} to '{to_status}' in Jira"),
                risk: self.risk(),
                payload: format!("{ticket} -> {to_status}"),
                sections: vec![],
            },
            Action::PostPrComment { repo, pr, comment } => ActionPreview {
                title: format!("Comment on {repo} PR #{pr}"),
                risk: self.risk(),
                payload: comment.body.clone(),
                sections: comment
                    .inline
                    .iter()
                    .map(|a| PreviewSection {
                        heading: "Inline".into(),
                        lines: vec![format!("{}:{} ({})", a.path, a.line, a.side.as_str())],
                    })
                    .collect(),
            },
            Action::ApprovePr { repo, pr } => ActionPreview {
                title: format!("Approve {repo} PR #{pr}"),
                risk: self.risk(),
                payload: format!("approve {repo} #{pr}"),
                sections: vec![],
            },
            Action::RequestChanges { repo, pr, body } => ActionPreview {
                title: format!("Request changes on {repo} PR #{pr}"),
                risk: self.risk(),
                payload: body.clone(),
                sections: vec![],
            },
            Action::TriggerPipeline { repo, spec } => ActionPreview {
                title: format!("Trigger a pipeline on {repo} ({})", spec.branch),
                risk: self.risk(),
                payload: format!(
                    "branch {}{}{}",
                    spec.branch,
                    spec.commit
                        .as_deref()
                        .map(|c| format!(" commit {c}"))
                        .unwrap_or_default(),
                    spec.custom_pipeline
                        .as_deref()
                        .map(|c| format!(" custom pipeline {c}"))
                        .unwrap_or_default()
                ),
                sections: vec![],
            },
            Action::RerunPipeline { repo, run_id } => ActionPreview {
                title: format!("Re-run pipeline {run_id} on {repo}"),
                risk: self.risk(),
                payload: format!("rerun {repo} {run_id}"),
                sections: vec![],
            },
        }
    }
}

fn preview_push(push: &PushUat) -> ActionPreview {
    let mut sections = Vec::new();
    let mut payload = Vec::new();
    for r in &push.repos {
        let mut lines = vec![
            format!(
                "branch: {} into {}/{}",
                r.ticket_branch, r.remote, r.uat_branch
            ),
            format!("uat before: {}", r.uat_before),
            format!("uat after:  {} (merge commit)", r.merge_commit),
            format!("{} commit(s) being merged:", r.commits.len()),
        ];
        lines.extend(
            r.commits
                .iter()
                .map(|c| format!("  {} {}", crate::git::short_sha(&c.sha), c.summary)),
        );
        lines.push(format!("{} file(s) changed:", r.files.len()));
        lines.extend(r.files.iter().map(|f| format!("  {f}")));
        sections.push(PreviewSection {
            heading: format!("{}:", r.repo),
            lines,
        });
        payload.push(format!(
            "({}) git push {} {}:refs/heads/{}",
            r.repo, r.remote, r.merge_commit, r.uat_branch
        ));
    }
    ActionPreview {
        title: format!(
            "Push {} to uat in {} repo(s); pushing uat DEPLOYS",
            push.ticket,
            push.repos.len()
        ),
        risk: Risk::High,
        payload: payload.join("\n"),
        sections,
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// An action, rendered for review. Nothing has been sent.
#[derive(Debug, Clone)]
pub struct Draft {
    id: String,
    action: Action,
    preview: ActionPreview,
    payload_hash: String,
    facts: Value,
}

impl Draft {
    pub(crate) fn new(action: Action, facts: Value) -> Self {
        let payload_hash = action.hash();
        let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            id: format!("draft-{seq}-{}", &payload_hash[..8]),
            preview: action.preview(),
            action,
            payload_hash,
            facts,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn action(&self) -> &Action {
        &self.action
    }

    pub fn preview(&self) -> &ActionPreview {
        &self.preview
    }

    pub fn payload_hash(&self) -> &str {
        &self.payload_hash
    }

    /// The human agreed to exactly this preview. Call it only after showing `preview()` and
    /// receiving an explicit yes; there is no other way to reach `Gateway::execute`.
    pub fn confirm(self) -> Confirmed {
        Confirmed {
            draft_id: self.id,
            action: self.action,
            payload_hash: self.payload_hash,
            facts: self.facts,
        }
    }
}

/// An action the human confirmed. Not `Clone`, consumed by `execute`: a confirmation is
/// used once and only for the payload that was previewed.
#[derive(Debug)]
pub struct Confirmed {
    pub(super) draft_id: String,
    pub(super) action: Action,
    pub(super) payload_hash: String,
    pub(super) facts: Value,
}

impl Confirmed {
    pub fn action(&self) -> &Action {
        &self.action
    }

    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    pub(super) fn facts_with_id(&self) -> Value {
        json!({ "draft": self.draft_id, "payload_hash": self.payload_hash, "facts": self.facts })
    }
}

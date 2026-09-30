//! Extra facts about a recorded `uat` merge (`state.db`, table `uat_merge_details`).

use eyre::Context;
use rusqlite::params;

use super::{Store, uat_merges::UatMerge};
use crate::domain::TicketKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    /// The app pushed this merge commit.
    Merge,
    /// The ticket was already in `uat`; the commit is the `uat` tip that contains it.
    AlreadyInUat,
}

impl MergeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeKind::Merge => "merge",
            MergeKind::AlreadyInUat => "already_in_uat",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "merge" => Some(Self::Merge),
            "already_in_uat" => Some(Self::AlreadyInUat),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeDetails {
    pub kind: MergeKind,
    pub ticket_branch: String,
    pub ticket_tip: String,
    pub uat_before: String,
}

/// A recorded merge with its details (absent for rows recorded before details existed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedMerge {
    pub merge: UatMerge,
    pub details: Option<MergeDetails>,
}

/// Records the merge and its details together (a repeat of the same commit is a no-op).
pub fn record(store: &Store, merge: &UatMerge, details: &MergeDetails) -> eyre::Result<()> {
    let tx = store.conn().unchecked_transaction()?;
    super::uat_merges::record(store, merge)?;
    let id: i64 = tx.query_row(
        "SELECT id FROM uat_merges WHERE ticket_key = ?1 AND repo = ?2 AND commit_sha = ?3",
        params![merge.ticket, merge.repo, merge.commit],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO uat_merge_details (merge_id, kind, ticket_branch, ticket_tip, uat_before)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id,
            details.kind.as_str(),
            details.ticket_branch,
            details.ticket_tip,
            details.uat_before
        ],
    )
    .wrap_err("Failed to record the uat merge details")?;
    tx.commit().wrap_err("Failed to record the uat merge")
}

/// Merges of a ticket, oldest first, with details.
pub fn list_for_ticket(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<DetailedMerge>> {
    let mut stmt = store.conn().prepare(
        "SELECT m.ticket_key, m.repo, m.branch, m.commit_sha, m.recorded_at,
                d.kind, d.ticket_branch, d.ticket_tip, d.uat_before
         FROM uat_merges m LEFT JOIN uat_merge_details d ON d.merge_id = m.id
         WHERE m.ticket_key = ?1 ORDER BY m.id",
    )?;
    let rows = stmt
        .query_map(params![ticket], |r| {
            let kind: Option<String> = r.get(5)?;
            let details = match kind {
                Some(k) => Some(MergeDetails {
                    kind: MergeKind::parse(&k).unwrap_or(MergeKind::Merge),
                    ticket_branch: r.get(6)?,
                    ticket_tip: r.get(7)?,
                    uat_before: r.get(8)?,
                }),
                None => None,
            };
            Ok(DetailedMerge {
                merge: UatMerge {
                    ticket: r.get(0)?,
                    repo: r.get(1)?,
                    branch: r.get(2)?,
                    commit: r.get(3)?,
                    recorded_at: r.get(4)?,
                },
                details,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read uat merges")?;
    Ok(rows)
}

/// The most recent merge of `ticket` in `repo`.
pub fn latest(
    store: &Store,
    ticket: &TicketKey,
    repo: &str,
) -> eyre::Result<Option<DetailedMerge>> {
    Ok(list_for_ticket(store, ticket)?
        .into_iter()
        .rfind(|m| m.merge.repo == repo))
}

//! Local ticket tracking (`state.db`): claims, status, ordering, kind override.

use eyre::{Context, bail, eyre};
use rusqlite::{OptionalExtension, Row, params};

use super::Store;
use crate::domain::{LocalStatus, TicketKey, TicketKind};

/// The local tracking row of a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketTracking {
    pub key: TicketKey,
    pub status: LocalStatus,
    /// Position in the single global manual order, 0-based.
    pub manual_order: i64,
    /// Manual override of the ticket kind; `None` means derive it from the PRs.
    pub kind_override: Option<TicketKind>,
    pub claimed_at: i64,
    pub updated_at: i64,
}

const COLUMNS: &str = "key, status, manual_order, kind_override, claimed_at, updated_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<TicketTracking> {
    Ok(TicketTracking {
        key: row.get(0)?,
        status: row.get(1)?,
        manual_order: row.get(2)?,
        kind_override: row.get(3)?,
        claimed_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

/// Starts tracking `key` as `Claimed`, last in the manual order. Errors if already tracked.
pub fn claim(store: &Store, key: &TicketKey, now: i64) -> eyre::Result<TicketTracking> {
    let inserted = store
        .conn()
        .execute(
            "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at)
             VALUES (?1, 'claimed', (SELECT COALESCE(MAX(manual_order) + 1, 0) FROM tickets), ?2, ?2)
             ON CONFLICT (key) DO NOTHING",
            params![key, now],
        )
        .wrap_err_with(|| format!("Failed to claim {key}"))?;
    if inserted == 0 {
        bail!("{key} is already tracked");
    }
    get(store, key)?.ok_or_else(|| eyre!("{key} vanished after being claimed"))
}

/// Stops tracking `key`, deleting its links, checklist, notes and time entries. Audit entries stay.
///
/// Refuses while an activation of the ticket is still recorded (restore points or an applied
/// overlay): deleting those rows would strand the repos on the ticket's branch and make the
/// test overlay permanent.
pub fn untrack(store: &Store, key: &TicketKey) -> eyre::Result<bool> {
    let pending: i64 = store.conn().query_row(
        "SELECT (SELECT COUNT(*) FROM activation_repos WHERE ticket_key = ?1)
              + (SELECT COUNT(*) FROM overlay_backups WHERE ticket_key = ?1)",
        params![key],
        |r| r.get(0),
    )?;
    if pending > 0 {
        bail!("{key} still has an activation to undo; deactivate it before untracking");
    }

    let removed = store
        .conn()
        .execute("DELETE FROM tickets WHERE key = ?1", params![key])
        .wrap_err_with(|| format!("Failed to untrack {key}"))?;
    Ok(removed > 0)
}

pub fn get(store: &Store, key: &TicketKey) -> eyre::Result<Option<TicketTracking>> {
    store
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM tickets WHERE key = ?1"),
            params![key],
            from_row,
        )
        .optional()
        .wrap_err_with(|| format!("Failed to read {key}"))
}

/// The one `Active` ticket, if any.
pub fn active(store: &Store) -> eyre::Result<Option<TicketTracking>> {
    store
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM tickets WHERE status = 'active'"),
            [],
            from_row,
        )
        .optional()
        .wrap_err("Failed to read the active ticket")
}

/// All tracked tickets in manual order.
pub fn list(store: &Store) -> eyre::Result<Vec<TicketTracking>> {
    query_list(
        store,
        &format!("SELECT {COLUMNS} FROM tickets ORDER BY manual_order, key"),
        params![],
    )
}

/// Tickets with `status`, in manual order.
pub fn list_by_status(store: &Store, status: LocalStatus) -> eyre::Result<Vec<TicketTracking>> {
    query_list(
        store,
        &format!("SELECT {COLUMNS} FROM tickets WHERE status = ?1 ORDER BY manual_order, key"),
        params![status],
    )
}

fn query_list(
    store: &Store,
    sql: &str,
    params: impl rusqlite::Params,
) -> eyre::Result<Vec<TicketTracking>> {
    let mut stmt = store.conn().prepare(sql)?;
    let rows = stmt
        .query_map(params, from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to list tickets")?;
    Ok(rows)
}

/// Moves `key` to `status`, validating the transition and the single-`Active` rule.
///
/// The database independently rejects a second `Active` ticket; the check here only gives
/// a readable error.
///
/// `Integrated` is refused here: it means "merged and pushed to `uat`" and only the
/// integration flow's finalize step may set it (see `mark_integrated`).
pub fn set_status(
    store: &Store,
    key: &TicketKey,
    to: LocalStatus,
    now: i64,
) -> eyre::Result<TicketTracking> {
    if to == LocalStatus::Integrated {
        bail!("{key} becomes 'integrated' only when it is merged and pushed to uat");
    }
    set_status_unguarded(store, key, to, now)
}

/// Sets `Integrated`. Crate-private on purpose: only `integration::finalize_integration`
/// (through `activation::deactivate_for_integration`) calls it, after every push succeeded.
pub(crate) fn mark_integrated(
    store: &Store,
    key: &TicketKey,
    now: i64,
) -> eyre::Result<TicketTracking> {
    set_status_unguarded(store, key, LocalStatus::Integrated, now)
}

fn set_status_unguarded(
    store: &Store,
    key: &TicketKey,
    to: LocalStatus,
    now: i64,
) -> eyre::Result<TicketTracking> {
    let current = get(store, key)?.ok_or_else(|| eyre!("{key} is not tracked"))?;
    if !current.status.can_transition_to(to) {
        bail!("{key} cannot go from {} to {to}", current.status);
    }
    if to == LocalStatus::Active
        && let Some(other) = active(store)?
    {
        bail!("{} is already active; park or finish it first", other.key);
    }

    store
        .conn()
        .execute(
            "UPDATE tickets SET status = ?2, updated_at = ?3 WHERE key = ?1",
            params![key, to, now],
        )
        .wrap_err_with(|| format!("Failed to set {key} to {to}"))?;
    get(store, key)?.ok_or_else(|| eyre!("{key} vanished while changing status"))
}

/// Sets or clears the manual ticket-kind override.
pub fn set_kind_override(
    store: &Store,
    key: &TicketKey,
    kind: Option<TicketKind>,
    now: i64,
) -> eyre::Result<()> {
    let changed = store
        .conn()
        .execute(
            "UPDATE tickets SET kind_override = ?2, updated_at = ?3 WHERE key = ?1",
            params![key, kind, now],
        )
        .wrap_err_with(|| format!("Failed to set the kind of {key}"))?;
    if changed == 0 {
        bail!("{key} is not tracked");
    }
    Ok(())
}

/// Moves `key` to `position` (0-based, clamped to the end) in the global manual order and
/// renumbers every ticket `0..n`, which also closes any gaps. Ties (which should not
/// exist) resolve by key so the result is deterministic.
pub fn reorder(store: &Store, key: &TicketKey, position: usize) -> eyre::Result<()> {
    let tx = store.conn().unchecked_transaction()?;

    let mut order: Vec<(TicketKey, i64)> = {
        let mut stmt =
            tx.prepare("SELECT key, manual_order FROM tickets ORDER BY manual_order, key")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };

    let from = order
        .iter()
        .position(|(k, _)| k == key)
        .ok_or_else(|| eyre!("{key} is not tracked"))?;
    let moved = order.remove(from);
    order.insert(position.min(order.len()), moved);

    for (index, (k, current)) in order.iter().enumerate() {
        let index = index as i64;
        if *current != index {
            tx.execute(
                "UPDATE tickets SET manual_order = ?2 WHERE key = ?1",
                params![k, index],
            )?;
        }
    }
    tx.commit().wrap_err("Failed to reorder tickets")
}

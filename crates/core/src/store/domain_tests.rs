//! Behavioural tests for the ticket persistence layer.

use serde_json::json;

use super::links::DiscoveredLink;
use super::*;
use crate::domain::{AuditOutcome, LocalStatus, RepoLinkOrigin, TicketKey, TicketKind};

fn key(s: &str) -> TicketKey {
    s.parse().unwrap()
}

fn state() -> Store {
    Store::open_in_memory(Kind::State).unwrap()
}

fn cache() -> Store {
    Store::open_in_memory(Kind::Cache).unwrap()
}

/// A store with the given tickets claimed at t=100 in that order.
fn state_with(keys: &[&str]) -> Store {
    let store = state();
    for k in keys {
        tickets::claim(&store, &key(k), 100).unwrap();
    }
    store
}

fn order(store: &Store) -> Vec<String> {
    tickets::list(store)
        .unwrap()
        .into_iter()
        .map(|t| t.key.to_string())
        .collect()
}

fn numbers(store: &Store) -> Vec<i64> {
    tickets::list(store)
        .unwrap()
        .into_iter()
        .map(|t| t.manual_order)
        .collect()
}

// --- schema ---------------------------------------------------------------------------

#[test]
fn migrations_apply_on_a_fresh_database() {
    let s = state();
    let c = cache();
    assert_eq!(s.schema_version().unwrap(), 2);
    assert_eq!(c.schema_version().unwrap(), 2);

    let tables = |store: &Store| -> Vec<String> {
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let state_tables = tables(&s);
    for t in [
        "tickets",
        "ticket_repos",
        "checklist_items",
        "notes",
        "time_entries",
        "audit_log",
    ] {
        assert!(state_tables.contains(&t.to_string()), "missing {t}");
    }
    assert!(tables(&c).contains(&"jira_tickets".to_string()));
    // Nothing leaks across the two databases.
    assert!(!tables(&c).contains(&"tickets".to_string()));
}

#[test]
fn upgrading_from_the_previous_schema_keeps_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");

    // A database as the previous release left it: version 1, one row.
    {
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        migrations::for_kind(Kind::State)
            .to_version(&mut conn, 1)
            .unwrap();
        conn.execute(
            "INSERT INTO app_meta (key, value) VALUES ('probe', 'kept')",
            [],
        )
        .unwrap();
    }

    let store = Store::open_in(dir.path(), Kind::State).unwrap();
    assert_eq!(store.schema_version().unwrap(), 2);
    let value: String = store
        .conn()
        .query_row("SELECT value FROM app_meta WHERE key = 'probe'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(value, "kept");
    // ... and the new tables work.
    tickets::claim(&store, &key("A-1"), 1).unwrap();
}

#[test]
fn check_constraints_match_the_enums() {
    let store = state_with(&["A-1"]);
    let conn = store.conn();

    for &status in LocalStatus::ALL {
        // Active is unique, so probe each value on a scratch row and delete it.
        conn.execute(
            "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at) VALUES ('Z-1', ?1, 9, 0, 0)",
            [status.as_str()],
        )
        .unwrap_or_else(|e| panic!("status {status} rejected: {e}"));
        conn.execute("DELETE FROM tickets WHERE key = 'Z-1'", [])
            .unwrap();
    }
    assert!(
        conn.execute(
            "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at) VALUES ('Z-1', 'bogus', 9, 0, 0)",
            [],
        )
        .is_err()
    );

    for &kind in TicketKind::ALL {
        conn.execute(
            "UPDATE tickets SET kind_override = ?1 WHERE key = 'A-1'",
            [kind.as_str()],
        )
        .unwrap();
    }
    assert!(
        conn.execute(
            "UPDATE tickets SET kind_override = 'urgent' WHERE key = 'A-1'",
            []
        )
        .is_err()
    );

    for &origin in RepoLinkOrigin::ALL {
        conn.execute(
            "INSERT INTO ticket_repos (ticket_key, repo, origin) VALUES ('A-1', ?1, ?1)",
            [origin.as_str()],
        )
        .unwrap();
    }
    assert!(
        conn.execute(
            "INSERT INTO ticket_repos (ticket_key, repo, origin) VALUES ('A-1', 'x', 'guess')",
            [],
        )
        .is_err()
    );

    for &outcome in AuditOutcome::ALL {
        conn.execute(
            "INSERT INTO audit_log (at, action, details, outcome) VALUES (0, 'a', '{}', ?1)",
            [outcome.as_str()],
        )
        .unwrap();
    }
    assert!(
        conn.execute(
            "INSERT INTO audit_log (at, action, details, outcome) VALUES (0, 'a', '{}', 'maybe')",
            [],
        )
        .is_err()
    );
}

#[test]
fn reopening_a_file_database_preserves_tickets() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open_in(dir.path(), Kind::State).unwrap();
        tickets::claim(&store, &key("PROJ-1"), 10).unwrap();
        tickets::claim(&store, &key("PROJ-2"), 11).unwrap();
        tickets::set_status(&store, &key("PROJ-2"), LocalStatus::Active, 12).unwrap();
        notes::add_note(&store, &key("PROJ-1"), "remember", 13).unwrap();
        time::start(&store, &key("PROJ-2"), 14).unwrap();
    }

    let store = Store::open_in(dir.path(), Kind::State).unwrap();
    assert_eq!(order(&store), ["PROJ-1", "PROJ-2"]);
    assert_eq!(tickets::active(&store).unwrap().unwrap().key, key("PROJ-2"));
    assert_eq!(
        notes::list_notes(&store, &key("PROJ-1")).unwrap()[0].body,
        "remember"
    );
    assert!(time::open_entry(&store).unwrap().is_some());
}

// --- tickets --------------------------------------------------------------------------

#[test]
fn claim_creates_tracking_with_next_order() {
    let store = state();
    let a = tickets::claim(&store, &key("a-1"), 50).unwrap();
    let b = tickets::claim(&store, &key("B-2"), 60).unwrap();

    assert_eq!(a.key, key("A-1"));
    assert_eq!(a.status, LocalStatus::Claimed);
    assert_eq!((a.manual_order, a.claimed_at, a.updated_at), (0, 50, 50));
    assert_eq!(b.manual_order, 1);
    assert_eq!(a.kind_override, None);

    let err = tickets::claim(&store, &key("A-1"), 70).unwrap_err();
    assert!(err.to_string().contains("already tracked"), "{err}");
    assert_eq!(
        tickets::get(&store, &key("A-1"))
            .unwrap()
            .unwrap()
            .claimed_at,
        50
    );
}

#[test]
fn set_status_follows_the_transition_table() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");

    let t = tickets::set_status(&store, &k, LocalStatus::Reviewing, 200).unwrap();
    assert_eq!((t.status, t.updated_at), (LocalStatus::Reviewing, 200));

    // Not allowed: Reviewing -> Integrated, and a no-op move.
    assert!(tickets::set_status(&store, &k, LocalStatus::Integrated, 201).is_err());
    assert!(tickets::set_status(&store, &k, LocalStatus::Reviewing, 201).is_err());
    let unchanged = tickets::get(&store, &k).unwrap().unwrap();
    assert_eq!(
        (unchanged.status, unchanged.updated_at),
        (LocalStatus::Reviewing, 200)
    );

    assert!(tickets::set_status(&store, &key("NOPE-1"), LocalStatus::Done, 1).is_err());
}

#[test]
fn full_lifecycle_walk() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");
    for status in [
        LocalStatus::Reviewing,
        LocalStatus::Active,
        LocalStatus::Parked,
        LocalStatus::Active,
        LocalStatus::Integrated,
        LocalStatus::Reviewing, // returned
        LocalStatus::Active,
        LocalStatus::Integrated,
        LocalStatus::Done,
        LocalStatus::Claimed, // came back to Review
    ] {
        tickets::set_status(&store, &k, status, 1).unwrap();
    }
}

#[test]
fn only_one_ticket_can_be_active() {
    let store = state_with(&["A-1", "B-2"]);
    tickets::set_status(&store, &key("A-1"), LocalStatus::Active, 1).unwrap();

    let err = tickets::set_status(&store, &key("B-2"), LocalStatus::Active, 2).unwrap_err();
    assert!(err.to_string().contains("A-1 is already active"), "{err}");

    // Parking the first frees the slot.
    tickets::set_status(&store, &key("A-1"), LocalStatus::Parked, 3).unwrap();
    tickets::set_status(&store, &key("B-2"), LocalStatus::Active, 4).unwrap();
    assert_eq!(tickets::active(&store).unwrap().unwrap().key, key("B-2"));
}

#[test]
fn database_rejects_a_second_active_ticket_even_without_the_rust_check() {
    let store = state_with(&["A-1", "B-2"]);
    let conn = store.conn();
    conn.execute("UPDATE tickets SET status = 'active' WHERE key = 'A-1'", [])
        .unwrap();

    let err = conn
        .execute("UPDATE tickets SET status = 'active' WHERE key = 'B-2'", [])
        .unwrap_err();
    assert!(err.to_string().contains("UNIQUE"), "{err}");

    let err = conn
        .execute(
            "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at) VALUES ('C-3', 'active', 2, 0, 0)",
            [],
        )
        .unwrap_err();
    assert!(err.to_string().contains("UNIQUE"), "{err}");

    // Many non-active tickets are fine.
    conn.execute("UPDATE tickets SET status = 'parked' WHERE key = 'A-1'", [])
        .unwrap();
    conn.execute("UPDATE tickets SET status = 'parked' WHERE key = 'B-2'", [])
        .unwrap();
}

#[test]
fn kind_override_can_be_set_and_cleared() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");

    tickets::set_kind_override(&store, &k, Some(TicketKind::Hotfix), 5).unwrap();
    let t = tickets::get(&store, &k).unwrap().unwrap();
    assert_eq!(
        (t.kind_override, t.updated_at),
        (Some(TicketKind::Hotfix), 5)
    );

    tickets::set_kind_override(&store, &k, None, 6).unwrap();
    assert_eq!(
        tickets::get(&store, &k).unwrap().unwrap().kind_override,
        None
    );
    assert!(tickets::set_kind_override(&store, &key("X-9"), None, 6).is_err());
}

#[test]
fn list_by_status_uses_manual_order() {
    let store = state_with(&["A-1", "B-2", "C-3", "D-4"]);
    for k in ["A-1", "C-3", "D-4"] {
        tickets::set_status(&store, &key(k), LocalStatus::Reviewing, 1).unwrap();
    }
    tickets::reorder(&store, &key("D-4"), 0).unwrap();

    let reviewing: Vec<String> = tickets::list_by_status(&store, LocalStatus::Reviewing)
        .unwrap()
        .into_iter()
        .map(|t| t.key.to_string())
        .collect();
    assert_eq!(reviewing, ["D-4", "A-1", "C-3"]);
    assert!(
        tickets::list_by_status(&store, LocalStatus::Done)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reorder_moves_and_renumbers() {
    let store = state_with(&["A-1", "B-2", "C-3", "D-4"]);

    // To first.
    tickets::reorder(&store, &key("C-3"), 0).unwrap();
    assert_eq!(order(&store), ["C-3", "A-1", "B-2", "D-4"]);
    assert_eq!(numbers(&store), [0, 1, 2, 3]);

    // To last (a position past the end clamps).
    tickets::reorder(&store, &key("C-3"), 99).unwrap();
    assert_eq!(order(&store), ["A-1", "B-2", "D-4", "C-3"]);

    // Same position changes nothing.
    tickets::reorder(&store, &key("B-2"), 1).unwrap();
    assert_eq!(order(&store), ["A-1", "B-2", "D-4", "C-3"]);
    assert_eq!(numbers(&store), [0, 1, 2, 3]);

    // Middle, moving down then up.
    tickets::reorder(&store, &key("A-1"), 2).unwrap();
    assert_eq!(order(&store), ["B-2", "D-4", "A-1", "C-3"]);
    tickets::reorder(&store, &key("C-3"), 1).unwrap();
    assert_eq!(order(&store), ["B-2", "C-3", "D-4", "A-1"]);

    assert!(tickets::reorder(&store, &key("NOPE-1"), 0).is_err());
}

#[test]
fn reorder_closes_gaps_and_handles_ties() {
    let store = state_with(&["A-1", "B-2", "C-3", "D-4"]);
    // Simulate an untracked ticket having left holes and a tie.
    store
        .conn()
        .execute_batch(
            "UPDATE tickets SET manual_order = 10 WHERE key = 'A-1';
             UPDATE tickets SET manual_order = 20 WHERE key = 'B-2';
             UPDATE tickets SET manual_order = 20 WHERE key = 'C-3';
             UPDATE tickets SET manual_order = 50 WHERE key = 'D-4';",
        )
        .unwrap();

    // Ties resolve by key: A, B, C, D. Moving D to the front renumbers 0..4.
    tickets::reorder(&store, &key("D-4"), 0).unwrap();
    assert_eq!(order(&store), ["D-4", "A-1", "B-2", "C-3"]);
    assert_eq!(numbers(&store), [0, 1, 2, 3]);
}

#[test]
fn claim_after_untrack_appends_after_the_max() {
    let store = state_with(&["A-1", "B-2", "C-3"]);
    assert!(tickets::untrack(&store, &key("B-2")).unwrap());
    assert!(!tickets::untrack(&store, &key("B-2")).unwrap());
    let d = tickets::claim(&store, &key("D-4"), 5).unwrap();
    assert_eq!(d.manual_order, 3);
    assert_eq!(order(&store), ["A-1", "C-3", "D-4"]);
}

// --- repo links -----------------------------------------------------------------------

fn found(repo: &str, branch: &str) -> DiscoveredLink {
    DiscoveredLink {
        repo: repo.into(),
        branch: Some(branch.into()),
    }
}

fn origins(store: &Store, k: &TicketKey) -> Vec<(String, Option<String>, RepoLinkOrigin)> {
    links::list(store, k)
        .unwrap()
        .into_iter()
        .map(|l| (l.repo, l.branch, l.origin))
        .collect()
}

#[test]
fn discovery_adds_refreshes_and_prunes_auto_links() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");

    links::apply_discovery(
        &store,
        &k,
        &[found("api", "feature/a-1"), found("web", "a-1-ui")],
    )
    .unwrap();
    assert_eq!(
        origins(&store, &k),
        [
            (
                "api".into(),
                Some("feature/a-1".into()),
                RepoLinkOrigin::Auto
            ),
            ("web".into(), Some("a-1-ui".into()), RepoLinkOrigin::Auto),
        ]
    );

    // web disappears, api's branch changes.
    links::apply_discovery(&store, &k, &[found("api", "feature/a-1-v2")]).unwrap();
    assert_eq!(
        origins(&store, &k),
        [(
            "api".into(),
            Some("feature/a-1-v2".into()),
            RepoLinkOrigin::Auto
        )]
    );

    // Applying the same result twice is idempotent.
    links::apply_discovery(&store, &k, &[found("api", "feature/a-1-v2")]).unwrap();
    assert_eq!(links::list(&store, &k).unwrap().len(), 1);
}

#[test]
fn discovery_never_clobbers_manual_or_excluded_links() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");

    links::add_manual(&store, &k, "api", Some("hand-picked")).unwrap();
    links::exclude(&store, &k, "old").unwrap();
    links::apply_discovery(
        &store,
        &k,
        &[
            found("api", "auto-branch"),
            found("old", "stale"),
            found("web", "w"),
        ],
    )
    .unwrap();

    assert_eq!(
        origins(&store, &k),
        [
            (
                "api".into(),
                Some("hand-picked".into()),
                RepoLinkOrigin::Manual
            ),
            ("old".into(), None, RepoLinkOrigin::Excluded),
            ("web".into(), Some("w".into()), RepoLinkOrigin::Auto),
        ]
    );

    // Discovery finding nothing removes only the auto link.
    links::apply_discovery(&store, &k, &[]).unwrap();
    let left: Vec<_> = origins(&store, &k).into_iter().map(|o| o.0).collect();
    assert_eq!(left, ["api", "old"]);

    let included: Vec<_> = links::list_included(&store, &k)
        .unwrap()
        .into_iter()
        .map(|l| l.repo)
        .collect();
    assert_eq!(included, ["api"]);
}

#[test]
fn manual_actions_override_and_remove_restores_discovery() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");

    links::apply_discovery(&store, &k, &[found("api", "auto")]).unwrap();
    links::exclude(&store, &k, "api").unwrap();
    assert_eq!(origins(&store, &k)[0].2, RepoLinkOrigin::Excluded);
    // Excluding an auto link keeps its branch.
    assert_eq!(origins(&store, &k)[0].1, Some("auto".into()));

    links::add_manual(&store, &k, "api", None).unwrap();
    assert_eq!(origins(&store, &k)[0].2, RepoLinkOrigin::Manual);

    assert!(links::remove(&store, &k, "api").unwrap());
    assert!(!links::remove(&store, &k, "api").unwrap());
    links::apply_discovery(&store, &k, &[found("api", "again")]).unwrap();
    assert_eq!(
        origins(&store, &k),
        [("api".into(), Some("again".into()), RepoLinkOrigin::Auto)]
    );
}

#[test]
fn links_require_a_tracked_ticket_and_cascade_on_untrack() {
    let store = state_with(&["A-1"]);
    assert!(links::add_manual(&store, &key("NOPE-1"), "api", None).is_err());

    links::add_manual(&store, &key("A-1"), "api", None).unwrap();
    notes::add_note(&store, &key("A-1"), "n", 1).unwrap();
    notes::add_checklist_item(&store, &key("A-1"), "c").unwrap();
    time::start(&store, &key("A-1"), 1).unwrap();
    tickets::untrack(&store, &key("A-1")).unwrap();

    for table in ["ticket_repos", "notes", "checklist_items", "time_entries"] {
        let n: i64 = store
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{table} should be empty");
    }
}

// --- checklist and notes --------------------------------------------------------------

#[test]
fn checklist_items_keep_order_and_toggle() {
    let store = state_with(&["A-1", "B-2"]);
    let a = key("A-1");

    let first = notes::add_checklist_item(&store, &a, "login works").unwrap();
    let second = notes::add_checklist_item(&store, &a, "export works").unwrap();
    notes::add_checklist_item(&store, &key("B-2"), "other ticket").unwrap();
    assert_eq!((first.position, second.position), (0, 1));
    assert!(!first.done);

    assert!(notes::toggle_checklist_item(&store, second.id).unwrap());
    let items = notes::list_checklist(&store, &a).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|i| (i.text.as_str(), i.done))
            .collect::<Vec<_>>(),
        [("login works", false), ("export works", true)]
    );
    assert!(!notes::toggle_checklist_item(&store, second.id).unwrap());

    notes::set_checklist_done(&store, first.id, true).unwrap();
    notes::set_checklist_done(&store, first.id, true).unwrap();
    assert!(notes::list_checklist(&store, &a).unwrap()[0].done);

    assert!(notes::toggle_checklist_item(&store, 9999).is_err());
    assert!(notes::set_checklist_done(&store, 9999, true).is_err());
    assert!(notes::add_checklist_item(&store, &key("NOPE-1"), "x").is_err());
}

#[test]
fn notes_list_oldest_first() {
    let store = state_with(&["A-1"]);
    let k = key("A-1");
    notes::add_note(&store, &k, "second", 20).unwrap();
    notes::add_note(&store, &k, "first", 10).unwrap();
    notes::add_note(&store, &k, "also second", 20).unwrap();

    let bodies: Vec<String> = notes::list_notes(&store, &k)
        .unwrap()
        .into_iter()
        .map(|n| n.body)
        .collect();
    assert_eq!(bodies, ["first", "second", "also second"]);
    assert!(notes::add_note(&store, &key("NOPE-1"), "x", 1).is_err());
}

// --- time -----------------------------------------------------------------------------

#[test]
fn time_start_stop_and_totals() {
    let store = state_with(&["A-1", "B-2"]);
    let (a, b) = (key("A-1"), key("B-2"));

    assert!(time::open_entry(&store).unwrap().is_none());
    assert!(time::stop(&store, 5).unwrap().is_none());
    assert_eq!(time::total_seconds(&store, &a, 5).unwrap(), 0);

    time::start(&store, &a, 1000).unwrap();
    assert_eq!(time::open_entry(&store).unwrap().unwrap().ticket, a);
    // A running entry counts up to `now`.
    assert_eq!(time::total_seconds(&store, &a, 1090).unwrap(), 90);
    // A `now` before the start never goes negative.
    assert_eq!(time::total_seconds(&store, &a, 900).unwrap(), 0);

    let closed = time::stop(&store, 1100).unwrap().unwrap();
    assert_eq!((closed.started_at, closed.ended_at), (1000, Some(1100)));
    assert!(time::open_entry(&store).unwrap().is_none());

    time::start(&store, &a, 2000).unwrap();
    time::stop(&store, 2050).unwrap();
    time::start(&store, &b, 3000).unwrap();
    assert_eq!(time::total_seconds(&store, &a, 9999).unwrap(), 150);
    assert_eq!(time::total_seconds(&store, &b, 3010).unwrap(), 10);
    assert_eq!(time::list(&store, &a).unwrap().len(), 2);
}

#[test]
fn double_start_is_rejected_by_code_and_by_the_database() {
    let store = state_with(&["A-1", "B-2"]);
    time::start(&store, &key("A-1"), 10).unwrap();

    let err = time::start(&store, &key("B-2"), 20).unwrap_err();
    assert!(err.to_string().contains("already running on A-1"), "{err}");
    assert!(time::start(&store, &key("A-1"), 20).is_err());

    // Bypass the Rust check.
    let err = store
        .conn()
        .execute(
            "INSERT INTO time_entries (ticket_key, started_at) VALUES ('B-2', 20)",
            [],
        )
        .unwrap_err();
    assert!(err.to_string().contains("UNIQUE"), "{err}");
    assert_eq!(time::list(&store, &key("B-2")).unwrap().len(), 0);
}

#[test]
fn stopping_before_the_start_closes_with_zero_length() {
    let store = state_with(&["A-1"]);
    time::start(&store, &key("A-1"), 500).unwrap();
    let closed = time::stop(&store, 400).unwrap().unwrap();
    assert_eq!(closed.ended_at, Some(500));
    assert_eq!(time::total_seconds(&store, &key("A-1"), 600).unwrap(), 0);
}

// --- audit ----------------------------------------------------------------------------

fn audit_entry(at: i64, action: &str, ticket: Option<&str>) -> audit::NewAuditEntry {
    audit::NewAuditEntry {
        at,
        action: action.into(),
        ticket: ticket.map(key),
        repo: ticket.map(|_| "api".into()),
        details: json!({ "sha": "abc123", "n": at }),
        outcome: AuditOutcome::Success,
    }
}

#[test]
fn audit_append_and_list() {
    let store = state();
    let id1 = audit::append(&store, &audit_entry(1, "git.push_uat", Some("A-1"))).unwrap();
    let id2 = audit::append(&store, &audit_entry(2, "jira.comment", Some("B-2"))).unwrap();
    audit::append(&store, &audit_entry(3, "sync", None)).unwrap();
    assert!(id2 > id1);

    // Audit entries do not need a tracked ticket.
    let all = audit::list(&store, None, 10).unwrap();
    assert_eq!(all.iter().map(|e| e.at).collect::<Vec<_>>(), [3, 2, 1]);
    assert_eq!(all[2].details, json!({ "sha": "abc123", "n": 1 }));
    assert_eq!(all[2].ticket, Some(key("A-1")));
    assert_eq!(all[2].repo.as_deref(), Some("api"));
    assert_eq!(all[0].ticket, None);

    let only_a = audit::list(&store, Some(&key("A-1")), 10).unwrap();
    assert_eq!(only_a.len(), 1);
    assert_eq!(audit::list(&store, None, 2).unwrap().len(), 2);
}

#[test]
fn audit_log_rejects_update_and_delete() {
    let store = state();
    let id = audit::append(&store, &audit_entry(1, "git.push_uat", Some("A-1"))).unwrap();

    let update = store
        .conn()
        .execute(
            "UPDATE audit_log SET outcome = 'failure' WHERE id = ?1",
            [id],
        )
        .unwrap_err();
    assert!(update.to_string().contains("append-only"), "{update}");
    let update_all = store
        .conn()
        .execute("UPDATE audit_log SET details = '{}'", [])
        .unwrap_err();
    assert!(
        update_all.to_string().contains("append-only"),
        "{update_all}"
    );

    let delete = store
        .conn()
        .execute("DELETE FROM audit_log WHERE id = ?1", [id])
        .unwrap_err();
    assert!(delete.to_string().contains("append-only"), "{delete}");
    assert!(store.conn().execute("DELETE FROM audit_log", []).is_err());

    // The entry is intact.
    let entries = audit::list(&store, None, 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].outcome, AuditOutcome::Success);
}

#[test]
fn audit_outlives_untracked_tickets() {
    let store = state_with(&["A-1"]);
    audit::append(&store, &audit_entry(1, "git.push_uat", Some("A-1"))).unwrap();
    tickets::untrack(&store, &key("A-1")).unwrap();
    assert_eq!(audit::list(&store, Some(&key("A-1")), 10).unwrap().len(), 1);
}

#[test]
fn audit_details_must_be_json() {
    let store = state();
    assert!(
        store
            .conn()
            .execute(
                "INSERT INTO audit_log (at, action, details, outcome) VALUES (0, 'a', 'not json', 'success')",
                [],
            )
            .is_err()
    );
}

// --- cache and views ------------------------------------------------------------------

fn jira(k: &str, title: &str, status: &str, at: i64) -> jira_cache::JiraTicket {
    jira_cache::JiraTicket {
        key: key(k),
        title: title.into(),
        jira_status: status.into(),
        priority: Some("High".into()),
        assignee: None,
        url: Some(format!("https://example.atlassian.net/browse/{k}")),
        raw_json: format!("{{\"key\":\"{k}\"}}"),
        fetched_at: at,
    }
}

#[test]
fn cache_upsert_is_idempotent_and_replaces() {
    let cache = cache();
    let t = jira("A-1", "First", "In Review", 10);

    jira_cache::upsert(&cache, &t).unwrap();
    jira_cache::upsert(&cache, &t).unwrap();
    assert_eq!(jira_cache::list(&cache).unwrap(), [t.clone()]);

    let newer = jira_cache::JiraTicket {
        title: "Renamed".into(),
        jira_status: "Returned".into(),
        priority: None,
        assignee: Some("sam".into()),
        url: None,
        fetched_at: 20,
        ..t
    };
    jira_cache::upsert(&cache, &newer).unwrap();
    assert_eq!(
        jira_cache::get(&cache, &key("A-1")).unwrap().unwrap(),
        newer
    );
    assert_eq!(jira_cache::list(&cache).unwrap().len(), 1);
    assert!(jira_cache::get(&cache, &key("Z-9")).unwrap().is_none());
}

#[test]
fn cache_lists_by_key() {
    let cache = cache();
    for k in ["C-3", "A-1", "B-2"] {
        jira_cache::upsert(&cache, &jira(k, "t", "In Review", 1)).unwrap();
    }
    let keys: Vec<String> = jira_cache::list(&cache)
        .unwrap()
        .into_iter()
        .map(|t| t.key.to_string())
        .collect();
    assert_eq!(keys, ["A-1", "B-2", "C-3"]);
}

#[test]
fn views_join_cache_and_tracking_from_either_side() {
    let state = state_with(&["B-2", "A-1"]); // tracked order: B-2, A-1
    let cache = cache();
    jira_cache::upsert(&cache, &jira("A-1", "Both", "In Review", 1)).unwrap();
    jira_cache::upsert(&cache, &jira("C-3", "Cache only", "In Review", 1)).unwrap();
    jira_cache::upsert(
        &cache,
        &jira("A-2", "Cache only, earlier key", "In Review", 1),
    )
    .unwrap();
    // B-2 is tracked but was never fetched (added manually).

    let both = jira_cache::view(&state, &cache, &key("A-1"))
        .unwrap()
        .unwrap();
    assert_eq!(both.jira.as_ref().unwrap().title, "Both");
    assert_eq!(both.tracking.as_ref().unwrap().status, LocalStatus::Claimed);

    let tracked_only = jira_cache::view(&state, &cache, &key("B-2"))
        .unwrap()
        .unwrap();
    assert!(tracked_only.jira.is_none() && tracked_only.tracking.is_some());

    let cache_only = jira_cache::view(&state, &cache, &key("C-3"))
        .unwrap()
        .unwrap();
    assert!(cache_only.jira.is_some() && cache_only.tracking.is_none());

    assert!(
        jira_cache::view(&state, &cache, &key("Z-9"))
            .unwrap()
            .is_none()
    );

    let all = jira_cache::list_views(&state, &cache).unwrap();
    let keys: Vec<String> = all.iter().map(|v| v.key.to_string()).collect();
    assert_eq!(keys, ["B-2", "A-1", "A-2", "C-3"]);
    assert!(all[1].jira.is_some() && all[1].tracking.is_some());
}

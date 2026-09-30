//! Tests of the tables the next-action engine added (migration 7).

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::{
    Kind, Store, drafts, migrations, reviews,
    suggestion_responses::{self as responses, ResponseKind, SuggestionResponse},
    tickets,
};
use crate::domain::TicketKey;

fn key() -> TicketKey {
    "PROJ-1".parse().unwrap()
}

fn response(id: &str, kind: ResponseKind, until: Option<i64>, at: i64) -> SuggestionResponse {
    SuggestionResponse {
        suggestion_id: id.into(),
        ticket: Some(key()),
        rule: "claim_new".into(),
        response: kind,
        reason: Some("r".into()),
        snooze_until: until,
        facts_hash: "h".into(),
        at,
    }
}

#[test]
fn the_next_action_migration_upgrades_a_v6_database_keeping_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut conn = Connection::open(dir.path().join("state.db")).unwrap();
        migrations::for_kind(Kind::State)
            .to_version(&mut conn, 6)
            .unwrap();
        conn.execute(
            "INSERT INTO tickets (key, status, manual_order, claimed_at, updated_at)
             VALUES ('PROJ-1', 'reviewing', 0, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO drafts (ticket_key, kind, body, status, created_at)
             VALUES ('PROJ-1', 'deploy_comment', 'b', 'draft', 1)",
            [],
        )
        .unwrap();
    }

    let store = Store::open_in(dir.path(), Kind::State).unwrap();
    assert_eq!(store.schema_version().unwrap(), 7);
    assert!(tickets::get(&store, &key()).unwrap().is_some());
    assert_eq!(drafts::list_for_ticket(&store, &key()).unwrap().len(), 1);
    // The new tables are empty and usable.
    assert!(responses::latest_map(&store).unwrap().is_empty());
    assert!(reviews::get(&store, &key()).unwrap().is_none());
    reviews::mark(&store, &key(), &BTreeMap::new(), 3).unwrap();
    assert_eq!(
        reviews::get(&store, &key()).unwrap().unwrap().reviewed_at,
        3
    );
}

#[test]
fn a_review_mark_round_trips_is_replaced_cleared_and_follows_the_ticket() {
    let store = Store::open_in_memory(Kind::State).unwrap();
    assert!(reviews::get(&store, &key()).unwrap().is_none());
    assert!(reviews::mark(&store, &key(), &BTreeMap::new(), 1).is_err());

    tickets::claim(&store, &key(), 1).unwrap();
    let heads = BTreeMap::from([("web".to_string(), "abc".to_string())]);
    reviews::mark(&store, &key(), &heads, 5).unwrap();
    let got = reviews::get(&store, &key()).unwrap().unwrap();
    assert_eq!((got.reviewed_at, got.heads), (5, heads));

    reviews::mark(&store, &key(), &BTreeMap::new(), 9).unwrap();
    assert_eq!(
        reviews::get(&store, &key()).unwrap().unwrap().reviewed_at,
        9
    );
    assert!(reviews::clear(&store, &key()).unwrap());
    assert!(!reviews::clear(&store, &key()).unwrap());

    reviews::mark(&store, &key(), &BTreeMap::new(), 9).unwrap();
    tickets::untrack(&store, &key()).unwrap();
    assert!(reviews::get(&store, &key()).unwrap().is_none());
}

#[test]
fn response_history_is_kept_and_the_latest_wins() {
    let store = Store::open_in_memory(Kind::State).unwrap();
    responses::record(&store, &response("a", ResponseKind::Dismissed, None, 1)).unwrap();
    responses::record(&store, &response("a", ResponseKind::Snoozed, Some(9), 2)).unwrap();
    responses::record(&store, &response("b", ResponseKind::Done, None, 3)).unwrap();

    assert_eq!(responses::history(&store, "a").unwrap().len(), 2);
    let latest = responses::latest_map(&store).unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(latest["a"].response, ResponseKind::Snoozed);
    assert_eq!(latest["a"].snooze_until, Some(9));
    assert_eq!(latest["b"].response, ResponseKind::Done);
}

#[test]
fn only_a_snooze_has_a_snooze_time() {
    let store = Store::open_in_memory(Kind::State).unwrap();
    assert!(responses::record(&store, &response("a", ResponseKind::Snoozed, None, 1)).is_err());
    assert!(
        responses::record(&store, &response("a", ResponseKind::Dismissed, Some(5), 1)).is_err()
    );
    assert!(
        store
            .conn()
            .execute(
                "INSERT INTO suggestion_responses (suggestion_id, rule, response, facts_hash, at)
                 VALUES ('a', 'r', 'bogus', 'h', 1)",
                [],
            )
            .is_err()
    );
}

#[test]
fn the_response_check_list_matches_the_enum() {
    let store = Store::open_in_memory(Kind::State).unwrap();
    for (i, kind) in ResponseKind::ALL.iter().enumerate() {
        let until = (*kind == ResponseKind::Snoozed).then_some(10);
        responses::record(&store, &response(&format!("s{i}"), *kind, until, 1)).unwrap();
    }
}

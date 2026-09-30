//! Schema migrations, one ordered list per database.
//!
//! Append only: never edit a migration once it has run on a real database. Anything new
//! (PRs, pipelines, overlays, ...) is added as a new entry at the end of the list.
//!
//! The CHECK lists below mirror the text forms of the enums in `crate::domain`; a test in
//! `store::tests` fails if they drift apart.

use std::sync::LazyLock;

use rusqlite_migration::{M, Migrations};

use super::Kind;

const APP_META: &str = "CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;";

/// Tickets and local tracking: everything here is irreplaceable.
const STATE_TICKETS: &str = "
CREATE TABLE tickets (
    key           TEXT PRIMARY KEY NOT NULL,
    status        TEXT NOT NULL
                  CHECK (status IN ('claimed','reviewing','active','parked','integrated','done')),
    manual_order  INTEGER NOT NULL,
    kind_override TEXT CHECK (kind_override IS NULL OR kind_override IN ('normal','hotfix')),
    claimed_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
) STRICT;

-- At most one Active ticket, whatever the application code does.
CREATE UNIQUE INDEX tickets_single_active ON tickets (status) WHERE status = 'active';
CREATE INDEX tickets_by_status_order ON tickets (status, manual_order);

CREATE TABLE ticket_repos (
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    repo       TEXT NOT NULL,
    branch     TEXT,
    origin     TEXT NOT NULL CHECK (origin IN ('auto','manual','excluded')),
    PRIMARY KEY (ticket_key, repo)
) STRICT;

CREATE TABLE checklist_items (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    text       TEXT NOT NULL,
    done       INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    UNIQUE (ticket_key, position)
) STRICT;

CREATE TABLE notes (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    body       TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX notes_by_ticket ON notes (ticket_key, created_at, id);

CREATE TABLE time_entries (
    id         INTEGER PRIMARY KEY,
    ticket_key TEXT NOT NULL REFERENCES tickets (key) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    ended_at   INTEGER CHECK (ended_at IS NULL OR ended_at >= started_at)
) STRICT;
CREATE INDEX time_entries_by_ticket ON time_entries (ticket_key);
-- At most one running timer overall.
CREATE UNIQUE INDEX time_entries_single_open ON time_entries ((1)) WHERE ended_at IS NULL;

-- No foreign key to tickets: the log must outlive the rows it describes.
CREATE TABLE audit_log (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    at         INTEGER NOT NULL,
    action     TEXT NOT NULL,
    ticket_key TEXT,
    repo       TEXT,
    details    TEXT NOT NULL CHECK (json_valid(details)),
    outcome    TEXT NOT NULL CHECK (outcome IN ('success','failure','skipped'))
) STRICT;
CREATE INDEX audit_log_by_ticket ON audit_log (ticket_key, id);

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
";

/// Mirror of Jira tickets; disposable.
const CACHE_JIRA: &str = "
CREATE TABLE jira_tickets (
    key         TEXT PRIMARY KEY NOT NULL,
    title       TEXT NOT NULL,
    jira_status TEXT NOT NULL,
    priority    TEXT,
    assignee    TEXT,
    url         TEXT,
    raw_json    TEXT NOT NULL,
    fetched_at  INTEGER NOT NULL
) STRICT;
";

static STATE: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::new(vec![M::up(APP_META), M::up(STATE_TICKETS)]));

static CACHE: LazyLock<Migrations<'static>> =
    LazyLock::new(|| Migrations::new(vec![M::up(APP_META), M::up(CACHE_JIRA)]));

pub(super) fn for_kind(kind: Kind) -> &'static Migrations<'static> {
    match kind {
        Kind::State => &STATE,
        Kind::Cache => &CACHE,
    }
}

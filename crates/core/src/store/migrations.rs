//! Schema migrations, one ordered list per database.
//!
//! Append only: never edit a migration once it has run on a real database. Anything the
//! domain model needs (tickets, repos, PRs, ...) is added as new entries, not here yet.

use std::sync::LazyLock;

use rusqlite_migration::{M, Migrations};

use super::Kind;

static STATE: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(vec![M::up(
        "CREATE TABLE app_meta (
            key   TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        ) STRICT;",
    )])
});

static CACHE: LazyLock<Migrations<'static>> = LazyLock::new(|| {
    Migrations::new(vec![M::up(
        "CREATE TABLE app_meta (
            key   TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        ) STRICT;",
    )])
});

pub(super) fn for_kind(kind: Kind) -> &'static Migrations<'static> {
    match kind {
        Kind::State => &STATE,
        Kind::Cache => &CACHE,
    }
}

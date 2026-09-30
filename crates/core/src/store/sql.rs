//! Maps domain types to and from SQLite text columns.

use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};

use crate::domain::{AuditOutcome, LocalStatus, RepoLinkOrigin, TicketKey, TicketKind};

macro_rules! text_column {
    ($($ty:ty),+) => {$(
        impl ToSql for $ty {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.as_str()))
            }
        }

        impl FromSql for $ty {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                value
                    .as_str()?
                    .parse()
                    .map_err(|e| FromSqlError::Other(Box::new(e)))
            }
        }
    )+};
}

text_column!(
    TicketKey,
    LocalStatus,
    TicketKind,
    RepoLinkOrigin,
    AuditOutcome
);

//! Domain types for the ticket-centred work desk.
//!
//! Plain data with validation and no I/O. Persistence lives in [`crate::store`].

mod status;
mod ticket_key;

pub use status::{
    AuditOutcome, BaselineChoice, LocalStatus, ParseEnumError, RepoLinkOrigin, TicketKind,
};
pub use ticket_key::{ParseTicketKeyError, TicketKey};

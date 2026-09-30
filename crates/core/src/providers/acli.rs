//! Jira adapter over Atlassian's `acli` (`acli jira workitem ...`).
//!
//! Empty on purpose: the `acli` adapter author fills this in. Implement
//! [`super::TicketProvider`] (and, separately, [`super::TicketWriter`]) here, then wire the
//! constructors in `registry.rs` (`build_ticket_provider` and `build_ticket_writer`).

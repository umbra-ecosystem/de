//! Provider contracts: the traits, data types and errors through which `de` talks to Jira and
//! Bitbucket, independent of how (CLI wrapper or REST).
//!
//! - [`model`]: data returned by providers.
//! - [`error`]: [`ProviderError`], which lets sync tell offline/auth problems from bugs.
//! - [`traits`]: [`TicketProvider`] and [`CodeHost`] (reads), [`TicketWriter`] and
//!   [`CodeHostWriter`] (writes, held only by the write gateway).
//! - [`registry`]: constructs adapters from config; the adapter authors' plug-in point.
//! - [`acli`], [`bkt`]: the real adapters (to be written).
//! - `fake`: in-memory implementations for tests (`cfg(test)` or the `testing` feature).

pub mod acli;
pub mod bkt;
pub mod error;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
pub mod model;
pub mod registry;
pub mod traits;

pub use error::{ProviderError, ProviderErrorKind, ProviderResult};
pub use model::{
    DiffSide, Health, InlineAnchor, NewPrComment, PipelineFilter, PipelineRun, PipelineState,
    PipelineStep, Pr, PrComment, PrFilter, PrState, RemoteComment, RemoteTicket, Reviewer,
    TriggerSpec, commit_matches,
};
pub use registry::Providers;
pub use traits::{CodeHost, CodeHostWriter, TicketProvider, TicketWriter};

//! The composer test overlay: pointing a consumer repo at the provider's checkout while a
//! ticket is being tested, and making sure it is undone and never pushed.
//!
//! - [`apply`] / [`revert`] change and restore the consumer's composer files (state kept in
//!   `state.db`, so a crash cannot make the overlay permanent).
//! - [`check_range_for_overlay`] / [`working_tree_has_overlay`] are the safety guards used
//!   before anything is integrated or pushed.
//! - Commands (composer, rebuild tasks) run through [`CommandRunner`] so tests can fake them.

pub mod composer;
mod engine;
mod guard;
mod runner;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use crate::domain::TicketKey;

pub use composer::{Finding, LeakKind};
pub use engine::{
    ApplyOutcome, ApplyRequest, OverlayPackage, RevertOutcome, apply, provider_url, revert,
};
pub use guard::{
    LeakedChange, OverlayGuardError, OverlayLeak, check_range_for_overlay,
    working_tree_has_overlay, working_tree_overlay,
};
pub use runner::{
    CommandOutput, CommandRunner, ExternalCommand, ProcessRunner, TimedOut, missing_dirs,
    run_checked, tool_fallback_dirs,
};

#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    /// An overlay is already recorded for this checkout; applying again would overwrite the
    /// backup of the original files.
    #[error("the overlay of {by} is already applied to {repo}; revert it first")]
    AlreadyApplied { by: TicketKey, repo: String },
    #[error("{0} has no composer.json")]
    MissingComposerJson(PathBuf),
    #[error("composer.json cannot be edited: {0}")]
    InvalidComposerJson(String),
    #[error("composer.json does not require {0}")]
    PackageNotRequired(String),
}

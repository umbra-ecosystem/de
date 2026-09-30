//! Activating a ticket for local test: planning which repos go to the ticket's branch and
//! which to a baseline, switching them safely, applying the composer overlay, and restoring
//! everything on deactivation.
//!
//! - [`plan_activation`] decides (pure).
//! - [`discover_links`] finds the ticket's branches across the workspace.
//! - [`activate`], [`deactivate`] and [`park`] carry it out, persisting restore points in
//!   `state.db` as they go.
//!
//! Commands run through [`crate::overlay::CommandRunner`]; times are passed in.

mod discovery;
mod engine;
mod plan;

#[cfg(test)]
mod tests;

pub use discovery::{Discovery, RepoMatches, WorkspaceRepo, discover_links, gather_plan_repos};
pub use engine::{
    ActivateOptions, ActivationReport, DeactivationReport, RepoActivation, RepoRestore,
    RestoreFailure, activate, deactivate, park, stash_label,
};
pub use plan::{
    ActivationPlan, Baselines, OverlayPlan, PlanError, PlanProblem, PlanRepo, RepoAction, RepoPlan,
    plan_activation,
};

/// Names of the audit entries written by activation and overlay handling.
pub mod actions {
    pub const LINKS_DISCOVERED: &str = "links.discovered";
    pub const ACTIVATION_START: &str = "activation.start";
    pub const ACTIVATION_REPO_SWITCHED: &str = "activation.repo_switched";
    pub const ACTIVATION_COMPLETE: &str = "activation.complete";
    pub const ACTIVATION_FAILED: &str = "activation.failed";
    pub const OVERLAY_APPLY: &str = "overlay.apply";
    pub const OVERLAY_REVERT: &str = "overlay.revert";
    pub const DEACTIVATION_REPO_RESTORED: &str = "deactivation.repo_restored";
    pub const DEACTIVATION_COMPLETE: &str = "deactivation.complete";
    pub const DEACTIVATION_INCOMPLETE: &str = "deactivation.incomplete";
}

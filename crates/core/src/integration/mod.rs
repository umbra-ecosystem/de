//! Integrating a ticket into `uat`: merge in a temporary worktree, guard, push through the
//! write gateway, record, then finalize.
//!
//! # Why the ticket must be Active
//!
//! Only a ticket that was activated (and therefore tested locally) is integrated: the merge
//! is of what you tested, and `Integrated` is reachable only from `Active`. The touched repos
//! are those whose activation record says the ticket branch was switched to; baseline
//! fallbacks are skipped. Finalize reverts the overlay and restores the branches.
//!
//! # The flow
//!
//! 1. [`prepare_integration`] pushes nothing. Per touched repo it fetches, builds the merge
//!    of the ticket branch into `origin/<uat>` in a temporary worktree under the data dir,
//!    then runs the overlay guard (by SHA), the optional `[integrate] checks`, and reports
//!    `Ready`, `UpToDate`, `Conflict` or `Blocked`.
//! 2. [`IntegrationPrep::push_action`] builds the `Action::PushUat`; the caller previews and
//!    confirms it through the [`Gateway`](crate::gateway::Gateway), which pushes and records
//!    each merge right after its push ([`execute_push`]).
//! 3. [`finalize_integration`] runs only once every touched repo is pushed (or was already
//!    in `uat`), and only it can make the ticket `Integrated`. If it fails partway (a repo
//!    cannot be restored) the ticket stays `Active` with its pushes recorded; preparing
//!    again resumes it (pushed repos are `AlreadyPushed`, and when every ticket repo was
//!    restored already the prep is empty, see `IntegrationPrep::is_resume`) and finalize
//!    runs again without pushing anything.
//! 4. [`cancel_integration`] removes the temporary worktrees (also done by the next prepare
//!    and by finalize).
//!
//! The user's local `uat` branch is never moved: the merge is built on a temporary branch
//! `de/integrate/<KEY>` started at `origin/<uat>`, and a local `uat` that has commits
//! `origin/<uat>` lacks makes the repo `Blocked`.

mod announce;
mod deploy;
mod prepare;
#[cfg(test)]
mod tests;

pub use announce::{
    ComposeOptions, compose_deploy_comment, discard_draft, post_comment, post_transition,
    preview_post_comment, preview_transition, update_draft_body,
};
pub use deploy::{DeployState, RepoDeploy, all_deployed, deploy_status, needs_remerge};
pub use prepare::{
    FinalizeReport, IntegrationPrep, ReadyMerge, RepoIntegration, RepoOutcome, cancel_integration,
    execute_push, finalize_integration, integration_dir, prepare_integration,
};

/// Names of the local (non-external) audit entries of the integration flow.
pub mod actions {
    pub const PREPARED: &str = "integration.prepared";
    pub const CANCELLED: &str = "integration.cancelled";
    pub const FINALIZED: &str = "integration.finalized";
    pub const ALREADY_IN_UAT: &str = "integration.already_in_uat";
    pub const DEPLOY_COMMENT_DRAFTED: &str = "deploy.comment_drafted";
    pub const DEPLOY_COMMENT_POSTED: &str = "deploy.comment_posted";
    pub const DEPLOY_TRANSITION_POSTED: &str = "deploy.transition_posted";
}

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::types::Slug;
use de_core::domain::{BaselineChoice, LocalStatus, TicketKey};
use de_core::sync::SyncSource;

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Increase verbosity for debugging purposes.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Initialize as a de project.
    Init {
        /// The path to the project directory. Defaults to the current directory.
        path: Option<PathBuf>,

        /// The name of the workspace of the project.
        #[arg(short, long)]
        workspace: Option<Slug>,

        /// The name of the project. Defaults to the current directory name.
        #[arg(short, long)]
        name: Option<Slug>,
    },

    /// Spin up projects. If workspace is provided, spins up all projects in the workspace.
    /// If no workspace is provided, spins up the current project and its dependencies.
    Start {
        #[arg(short, long)]
        workspace: Option<Option<Slug>>,

        /// Skip confirmation prompts and proceed with starting.
        #[arg(short, long)]
        yes: bool,
    },

    /// Spin down all projects in the workspace.
    Stop {
        /// The name of the workspace to stop projects in. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,

        /// Skip the confirmation about uncommitted or unpushed work.
        #[arg(short, long)]
        yes: bool,
    },

    /// Inspect the git state of the projects in a workspace.
    Git {
        #[command(subcommand)]
        command: GitCommands,
    },

    /// Track tickets and test one at a time: claim, activate, park, notes and checklist.
    Ticket {
        #[command(subcommand)]
        command: TicketCommands,
    },

    /// Refresh the local cache of tickets, PRs and pipelines from Jira and Bitbucket.
    ///
    /// Only reads from the remote systems. Sources that failed (offline, not logged in) are
    /// reported and their cached data is kept.
    Sync {
        /// Sync only this source.
        #[arg(long, value_enum)]
        only: Option<SyncOnly>,

        /// Sync even sources that were synced very recently.
        #[arg(long)]
        force: bool,
    },

    /// Show what to do next: ranked suggestions from the state of your tickets.
    ///
    /// Suggestions are computed from the local cache (run `de sync` first, or do the
    /// suggested sync). Nothing is changed by listing them. `de next do <id>` carries one
    /// out: local steps after a confirmation, and every external write only after an exact
    /// preview and an explicit yes (there is no flag that skips it).
    Next {
        #[command(subcommand)]
        command: Option<NextCommands>,

        /// The workspace whose repos to look at. Defaults to the active workspace.
        #[arg(short, long, global = true)]
        workspace: Option<Slug>,

        /// List every suggestion, including dismissed and snoozed ones (marked), not only the top few.
        #[arg(long)]
        all: bool,

        /// Print the machine-readable form (versioned JSON) instead of text.
        #[arg(long)]
        json: bool,
    },

    /// Inspect the external tools (`acli`, `bkt`) that provide Jira and Bitbucket data.
    Providers {
        #[command(subcommand)]
        command: ProvidersCommands,
    },

    /// Run `docker compose` for a project or every project in a workspace.
    ///
    /// Everything after `--` is passed to `docker compose` unchanged, e.g.
    /// `de compose -- logs -f api`.
    Compose {
        /// The name of the project to target. Defaults to the current project.
        #[arg(short, long, conflicts_with = "workspace")]
        project: Option<Slug>,

        /// Run against every project in the workspace, in dependency order.
        /// Uses the active workspace when no name is given.
        #[arg(short, long)]
        workspace: Option<Option<Slug>>,

        /// Arguments passed to `docker compose`.
        #[arg(last = true, required = true)]
        args: Vec<String>,
    },

    /// Run a command in the context of the current project.
    Run {
        /// The command to run listed in config file.
        command: Slug,

        /// The name of the project to run the command in. Defaults to the current project.
        #[arg(short, long)]
        project: Option<Slug>,

        /// The name of the workspace to run the command in. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,

        /// Additional arguments to pass to the command.
        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Execute a command in a project's context.
    Exec {
        /// The name of the project to execute the command in. If not provided, uses the current/inferred project.
        #[clap(short, long)]
        project: Option<Slug>,

        /// The name of the workspace to execute the command in. Defaults to the active workspace.
        #[clap(short, long)]
        workspace: Option<Slug>,

        /// The command to execute.
        #[clap(last = true)]
        command: Vec<String>,
    },

    /// Execute a command in the context of all projects in a workspace.
    ExecAll {
        /// The name of the workspace to execute the command in. Defaults to the active workspace.
        #[clap(short, long)]
        workspace: Option<Slug>,

        /// The command to execute.
        #[clap(last = true)]
        command: Vec<String>,
    },

    /// List all projects of the current workspace.
    List {
        /// The name of the workspace to list projects from. Defaults to the current workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Scan de projects and update the workspace configs.
    Scan {
        /// The directory to discover projects in.
        dir: Option<PathBuf>,

        /// The name of the workspace to discover projects in. Defaults to all workspaces.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Update workspace registrations and project configurations.
    Update {
        /// Update all workspaces and projects.
        #[arg(long)]
        all: bool,

        /// The name of the workspace to update projects in. Defaults to the current workspace.
        #[arg(short, long)]
        workspace: Option<Option<Slug>>,
    },

    /// Manage tasks defined in the project.
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// Manage the de CLI itself.
    #[command(name = "self")]
    Self_ {
        #[command(subcommand)]
        command: SelfCommands,
    },

    /// Manage workspace-level operations.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommands,
    },

    /// Manage the configuration of the de CLI.
    Config {
        /// The property key to set or get (e.g., "active").
        key: String,

        /// The value to set for the property. If omitted, prints the current value.
        value: Option<String>,

        /// Whether to unset the property instead of setting it.
        #[arg(short, long)]
        unset: bool,
    },

    #[command(external_subcommand)]
    Fallthrough(Vec<String>),
}

#[derive(Debug, Subcommand)]
pub enum TaskCommands {
    /// List all tasks defined in the project.
    List,

    /// Add a task to the project or workspace configuration.
    Add {
        /// The name of the task to add.
        task: Slug,

        /// The command to execute for the task.
        task_command: String,

        /// The name of the project to add the task to. Defaults to the current project.
        #[clap(short, long)]
        project: Option<Slug>,

        /// Add the task to the workspace configuration instead of the project.
        #[clap(short, long)]
        workspace: Option<Option<Slug>>,
    },

    /// Remove a task from the project or workspace configuration.
    Remove {
        /// The name of the task to remove.
        task: Slug,

        /// The name of the project to remove the task from. Defaults to the current project.
        #[clap(short, long)]
        project: Option<Slug>,

        /// Remove the task from the workspace configuration instead of the project.
        #[clap(short, long)]
        workspace: Option<Option<Slug>>,
    },
}

#[derive(Debug, Subcommand)]
pub enum GitCommands {
    /// Show branch, changes, ahead/behind and upstream of every project (read-only).
    Status {
        /// The name of the workspace to inspect. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TicketCommands {
    /// Start tracking a ticket.
    Claim {
        /// The ticket key, e.g. PROJ-123.
        key: TicketKey,

        /// A title to show next to the key.
        #[arg(long)]
        title: Option<String>,

        /// Mark the ticket as a hotfix: activating it asks which baseline untouched repos use.
        #[arg(long)]
        hotfix: bool,
    },

    /// List the tracked tickets.
    List,

    /// Show a ticket: status, repos and branches, checklist, notes and time.
    Show {
        key: TicketKey,

        /// The workspace whose repos to look at. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Choose the branch a ticket uses in a repo, or hide the repo from the ticket.
    Link {
        key: TicketKey,

        /// The workspace project (repo) name.
        repo: String,

        /// The branch to use, overriding discovery. Without it the repo is linked and its
        /// matching branch is discovered.
        #[arg(long, conflicts_with = "exclude")]
        branch: Option<String>,

        /// Hide the repo from the ticket (a stale or duplicate branch).
        #[arg(long)]
        exclude: bool,

        /// The workspace the repo belongs to. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Make a ticket the active one: switch repos to its branches and apply the test overlay.
    Activate {
        key: TicketKey,

        /// Where repos the ticket does not touch go: base, production or uat. Only used for
        /// hotfixes; without it a hotfix asks.
        #[arg(long)]
        baseline: Option<BaselineChoice>,

        /// `git fetch` every repo first.
        #[arg(long)]
        fetch: bool,

        /// The workspace to activate in. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Set the active ticket aside, restoring the repos and reverting the overlay.
    Park {
        /// The ticket. Defaults to the active one, or the one an interrupted activation
        /// left work to undo for.
        key: Option<TicketKey>,
    },

    /// Deactivate the active ticket, restoring the repos and reverting the overlay.
    Deactivate {
        /// The ticket. Defaults to the active one, or the one an interrupted activation
        /// left work to undo for.
        key: Option<TicketKey>,

        /// The status to move the ticket to. Defaults to parked.
        #[arg(long)]
        status: Option<LocalStatus>,
    },

    /// Add a note to a ticket.
    Note { key: TicketKey, text: String },

    /// Merge the active ticket into uat in a temporary worktree and, after you confirm,
    /// push it (pushing uat deploys). There is no way to skip the confirmation.
    Integrate {
        key: TicketKey,

        /// Prepare and show exactly what would be pushed, then stop. Never asks, never pushes.
        #[arg(long)]
        dry_run: bool,

        /// The workspace whose repos to integrate. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Show the integration state of a ticket: touched repos and recorded uat merges.
    Integration { key: TicketKey },

    /// Show whether each repo's uat merge has been deployed (from the last `de sync`).
    Deploy {
        key: TicketKey,

        /// The workspace whose repos to look at. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Draft the Jira deploy comment (saved locally, nothing is posted).
    DraftComment {
        key: TicketKey,

        /// Allow a comment while some repos are not deployed; it says which.
        #[arg(long)]
        partial: bool,

        /// Text to append to the comment.
        #[arg(long)]
        note: Option<String>,

        /// The workspace whose repos to look at. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Show the drafted deploy comment and, after you confirm, post it to Jira.
    PostComment { key: TicketKey },

    /// After the comment is posted: show and, after you confirm, move the ticket to the
    /// alpha-testing status in Jira.
    Transition { key: TicketKey },

    /// Remove the temporary integration worktrees of a ticket.
    CancelIntegration {
        key: TicketKey,

        /// The workspace whose repos to clean. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Manage the test checklist of a ticket.
    Check {
        #[command(subcommand)]
        command: CheckCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum NextCommands {
    /// Hide a suggestion until its facts change materially.
    Dismiss {
        /// The suggestion id, as printed by `de next`.
        id: String,

        /// Why (kept, to tune the rules later).
        #[arg(long)]
        reason: Option<String>,
    },

    /// Hide a suggestion for a while.
    Snooze {
        /// The suggestion id, as printed by `de next`.
        id: String,

        /// How long: a number and a unit, e.g. 30m, 2h, 1d, 1w.
        #[arg(long = "for", value_name = "DURATION")]
        duration: String,
    },

    /// Carry out a suggestion.
    Do {
        /// The suggestion id, as printed by `de next`.
        id: String,
    },
}

/// The sources `de sync --only` accepts.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SyncOnly {
    Jira,
    Bitbucket,
}

impl From<SyncOnly> for SyncSource {
    fn from(only: SyncOnly) -> Self {
        match only {
            SyncOnly::Jira => SyncSource::Jira,
            SyncOnly::Bitbucket => SyncSource::Bitbucket,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ProvidersCommands {
    /// Show whether each provider is installed, logged in and recent enough.
    Check,

    /// Run harmless read-only commands against `acli` and `bkt` and print their raw output.
    ///
    /// Use this on a real install to confirm the output shapes `de` expects, and paste the
    /// result into a bug report. The output can contain ticket and PR titles: read it before
    /// sharing it.
    Probe {
        /// A Jira key to view and list comments for (e.g. PROJ-123).
        #[arg(long)]
        ticket: Option<TicketKey>,

        /// A Bitbucket repository as `workspace/slug` to list PRs and pipelines for.
        #[arg(long)]
        repo: Option<String>,

        /// Write the report to this file instead of printing it.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum CheckCommands {
    /// Add an item to the end of the checklist.
    Add { key: TicketKey, text: String },

    /// Tick or untick an item, by the number `list` shows.
    Toggle { key: TicketKey, number: usize },

    /// Show the checklist.
    List { key: TicketKey },
}

#[derive(Debug, Subcommand)]
pub enum SelfCommands {
    /// Update the de CLI itself.
    Update,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommands {
    /// Run a task defined in the workspace configuration.
    Run {
        /// The name of the task to run.
        task: Slug,

        /// The name of the workspace to run the task in. Defaults to the active workspace.
        #[clap(short, long)]
        workspace: Option<Slug>,

        /// Additional arguments to pass to the task command.
        #[clap(hide = true)]
        args: Vec<String>,
    },

    /// Set or get a property on the workspace (e.g., active, default-branch).
    Config {
        /// The name of the workspace to modify. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,

        /// The property key to set or get (e.g., "active", "default-branch").
        key: String,

        /// The value to set for the property. If omitted, prints the current value.
        value: Option<String>,

        /// Whether to unset the property instead of setting it.
        #[arg(short, long)]
        unset: bool,
    },

    /// Get information about a workspace.
    Info {
        /// The name of the workspace to get information about. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },
}

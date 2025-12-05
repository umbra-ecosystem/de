use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::types::Slug;

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

        /// Skip confirmation prompts and proceed with stopping.
        #[arg(short, long)]
        yes: bool,
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
        /// The name of the project to execute the command in.
        project: Slug,

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

    /// Manage the workspace setup and configuration.
    Setup {
        /// The snapshot file to create or apply to the workspace.
        snapshot: PathBuf,

        /// The directory to apply the snapshot to. Defaults to the current directory.
        #[arg(short, long)]
        target_dir: Option<PathBuf>,
    },

    /// Manage tasks defined in the project.
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// Manage shims for de commands.
    #[cfg(target_family = "unix")]
    Shim {
        #[command(subcommand)]
        command: ShimCommands,
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

    /// Diagnose and check the health of your de environment.
    Doctor {
        /// The name of the workspace to diagnose. Defaults to the active workspace.
        workspace: Option<Slug>,
    },

    /// Show the status of the current workspace and projects.
    Status {
        /// The name of the workspace to show status for. Defaults to the active workspace.
        workspace: Option<Slug>,
    },

    /// Manage git repositories in the workspace.
    Git {
        #[command(subcommand)]
        command: GitCommands,
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
pub enum GitCommands {
    /// Switch branches in all projects in the workspace.
    Switch {
        /// The branch to switch to.
        target_branch: String,

        /// The branch to fallback to if the target branch does not exist.
        #[arg(short, long)]
        fallback: Option<String>,

        /// What to do if there are uncommitted changes.
        #[arg(long)]
        on_dirty: Option<OnDirtyAction>,
    },

    /// Reset all projects to a clean state on a base branch before starting new work.
    BaseReset {
        /// The base branch to reset to. Defaults to the workspace's default branch or 'dev'.
        base_branch: Option<String>,

        /// What to do if there are uncommitted changes.
        #[arg(short = 'd', long, value_enum, default_value_t = OnDirtyAction::Prompt)]
        on_dirty: OnDirtyAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnDirtyAction {
    /// Prompt the user for action.
    Prompt,
    /// Stash changes and proceed.
    Stash,
    /// Force checkout (discard all changes).
    Force,
    /// Abort the operation.
    Abort,
}

#[derive(Debug, Subcommand)]
pub enum TaskCommands {
    /// Check if a task is defined in the project.
    Check {
        /// The name of the task to check.
        task: Slug,
    },

    /// List all tasks defined in the project.
    List,

    /// Add a task to the project or workspace configuration.
    Add {
        /// The name of the task to add.
        task: Slug,

        /// The command to execute for the task.
        task_command: String,

        /// The Docker Compose service to execute the command in (for project tasks).
        #[clap(short, long)]
        service: Option<String>,

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
pub enum ShimCommands {
    /// Add a shim for a specific command.
    Add {
        /// The command to shim.
        command: Slug,
    },

    /// Remove a shim for a specific command.
    Remove {
        /// The command to remove the shim for.
        command: Slug,
    },

    /// Update the current shims to the latest version.
    Reinstate,

    /// List all shims currently installed.
    List,

    /// Add de shims directory to the PATH.
    Install,

    /// Uninstall the de shims directory from the PATH.
    Uninstall,
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

    Snapshot {
        /// The name of the workspace to create a snapshot for. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,

        /// The profile to use for the snapshot. Defaults to "default".
        #[arg(short, long, default_value = "default")]
        profile: Slug,
    },
}

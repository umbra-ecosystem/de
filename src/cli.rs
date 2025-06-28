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

    /// Spin up all projects in the workspace.
    Start {
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Spin down all projects in the workspace.
    Stop {
        /// The name of the workspace to stop projects in. Defaults to the active workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Run a command in the context of the current project.
    Run {
        /// The command to run listed in config file.
        command: Slug,

        /// Additional arguments to pass to the command.
        #[arg(allow_hyphen_values = true, hide = true)]
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

    /// Manage tasks defined in the project.
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// Manage shims for de commands.
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

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

    /// List all projects of the current workspace.
    List {
        /// The name of the workspace to list projects from. Defaults to the current workspace.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },

    /// Scan de projects and update the workspace configs.
    Scan {
        /// The directory to discover projects in.
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// The name of the workspace to discover projects in. Defaults to all workspaces.
        #[arg(short, long)]
        workspace: Option<Slug>,
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
    /// Add de shims directory to the PATH.
    Install,

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
}

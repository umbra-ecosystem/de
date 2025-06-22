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
        /// The name of the workspace of the project.
        workspace: Slug,
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

    /// Discover de projects and update the cache.
    Discover {
        /// The directory to discover projects in.
        #[arg(short, long, default_value = ".")]
        directory: PathBuf,

        /// The name of the workspace to discover projects in. Defaults to all workspaces.
        #[arg(short, long)]
        workspace: Option<Slug>,
    },
}

//! Runs the `git` binary for everything that mutates a repository or talks to
//! a remote, so the user's SSH agent, credential helpers, hooks and config apply.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use eyre::Context;

/// A failed `git` invocation, carrying the command line and what git said.
#[derive(Debug, thiserror::Error)]
#[error("`git {args}` failed in {dir} ({status}): {stderr}")]
pub struct GitCommandError {
    pub args: String,
    pub dir: String,
    pub status: String,
    pub stderr: String,
}

/// Captured result of a `git` invocation.
#[derive(Debug, Clone)]
pub struct GitOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `git` with a fixed working directory.
#[derive(Debug, Clone)]
pub struct GitRunner {
    dir: PathBuf,
}

impl GitRunner {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Run git and return the captured output whether or not it succeeded.
    /// Only fails when git cannot be spawned at all.
    pub fn run_raw(&self, args: &[&str]) -> eyre::Result<GitOutput> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            // Never block on a credential prompt; fail with a message instead.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .output()
            .wrap_err_with(|| format!("Failed to spawn `git {}`", args.join(" ")))?;

        Ok(GitOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Run git and return trimmed stdout, erroring with the command and stderr on failure.
    pub fn run(&self, args: &[&str]) -> eyre::Result<String> {
        let output = self.run_raw(args)?;
        if output.success {
            return Ok(output.stdout.trim_end().into());
        }

        Err(GitCommandError {
            args: args.join(" "),
            dir: self.dir.display().to_string(),
            status: "non-zero exit".into(),
            stderr: output.stderr.trim().into(),
        }
        .into())
    }
}

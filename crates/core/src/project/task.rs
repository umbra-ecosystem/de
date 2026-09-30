use std::{
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

use crate::project::task_detector::TaskSource;

/// A task defined in `de.toml`, either as a bare command string or as a table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Task {
    Flat(String),
    Complex { command: String },
}

impl Task {
    pub fn command_str(&self) -> &str {
        match self {
            Task::Flat(cmd) => cmd,
            Task::Complex { command } => command,
        }
    }
}

/// Where a resolved task came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOrigin {
    /// Defined under `[tasks]` in `de.toml`.
    Configured,
    /// Found in a project file such as `package.json` or a `Makefile`.
    Detected(TaskSource),
}

/// A task ready to be run: the command line and the directory to run it in.
#[derive(Debug, Clone)]
pub struct ResolvedTask {
    pub command: String,
    pub dir: PathBuf,
    pub origin: TaskOrigin,
}

impl ResolvedTask {
    /// Builds the process for this task, appending `args` to the command line.
    pub fn to_command(&self, args: &[String]) -> Command {
        shell_command(&self.command, &self.dir, args)
    }
}

/// Builds `sh -c '<command> "$@"' de-task <args...>` in `dir`.
///
/// Going through the shell lets task commands use quoting, pipes, `&&` and
/// environment variables, while `args` reach the command as separate words.
pub fn shell_command(command: &str, dir: &Path, args: &[String]) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!("{command} \"$@\""))
        .arg("de-task")
        .args(args)
        .current_dir(dir);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(command: &str, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = shell_command(command, Path::new("."), &args)
            .output()
            .expect("sh should run");
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn passes_args_as_separate_words() {
        assert_eq!(run("printf '[%s]'", &["a b", "c"]), "[a b][c]");
    }

    #[test]
    fn keeps_quotes_in_the_command() {
        assert_eq!(run("printf '%s' 'x  y'", &[]), "x  y");
    }

    #[test]
    fn supports_shell_operators() {
        assert_eq!(run("echo one && echo two", &[]), "one\ntwo\n");
    }
}

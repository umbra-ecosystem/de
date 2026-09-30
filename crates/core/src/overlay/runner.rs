//! Running external commands (composer, rebuild tasks) behind a trait, so tests can
//! substitute a fake while the real implementation runs the process in the repo directory.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use eyre::{Context, eyre};

/// A command to run in a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCommand {
    /// Directory the command runs in (the repo).
    pub dir: PathBuf,
    pub program: String,
    pub args: Vec<String>,
    /// What to call it in messages, e.g. `composer update acme/api-client` or `task build-ui`.
    pub label: String,
}

impl ExternalCommand {
    pub fn new(
        dir: impl Into<PathBuf>,
        program: impl Into<String>,
        args: &[&str],
        label: impl Into<String>,
    ) -> Self {
        Self {
            dir: dir.into(),
            program: program.into(),
            args: args.iter().map(|a| String::from(*a)).collect(),
            label: label.into(),
        }
    }

    /// A composer invocation in `dir`, labelled with its arguments.
    pub fn composer(dir: &Path, args: &[&str]) -> Self {
        Self::new(
            dir,
            "composer",
            args,
            format!("composer {}", args.join(" ")),
        )
    }

    /// Capture a prepared [`Command`] (such as the shell command of a resolved task).
    pub fn from_command(command: &Command, dir: &Path, label: impl Into<String>) -> Self {
        Self {
            dir: command
                .get_current_dir()
                .map_or_else(|| dir.into(), Path::to_path_buf),
            program: command.get_program().to_string_lossy().into_owned(),
            args: command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    /// Exit code; `None` when the process was killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs external commands. Only fails when the process cannot be started at all; a command
/// that ran and failed comes back as `Ok` with `success == false` (see [`run_checked`]).
pub trait CommandRunner {
    fn run(&self, command: &ExternalCommand) -> eyre::Result<CommandOutput>;
}

/// Run `command` and turn a non-zero exit into an error carrying what it printed.
pub fn run_checked(
    runner: &dyn CommandRunner,
    command: &ExternalCommand,
) -> eyre::Result<CommandOutput> {
    let output = runner.run(command).wrap_err_with(|| {
        format!(
            "Failed to run `{}` in {}",
            command.label,
            command.dir.display()
        )
    })?;
    if output.success {
        return Ok(output);
    }

    let detail = tail(&output.stderr)
        .or_else(|| tail(&output.stdout))
        .unwrap_or_default();
    Err(eyre!(
        "`{}` failed in {} (exit {}){}{}",
        command.label,
        command.dir.display(),
        output
            .code
            .map_or_else(|| "signal".into(), |c| c.to_string()),
        if detail.is_empty() { "" } else { ": " },
        detail
    ))
}

/// The last few non-empty lines of `text`, if any.
fn tail(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let start = lines.len().saturating_sub(5);
    Some(lines[start..].join("\n"))
}

/// Runs the process for real, with the command's directory as its working directory.
#[derive(Debug, Clone, Default)]
pub struct ProcessRunner {
    /// Directories put in front of `PATH` for the child (tests use this for a stub `composer`).
    path_prefix: Vec<PathBuf>,
}

impl ProcessRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look for programs in `dir` before anything already on `PATH`.
    pub fn with_path_prefix(mut self, dir: impl Into<PathBuf>) -> Self {
        self.path_prefix.push(dir.into());
        self
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&self, command: &ExternalCommand) -> eyre::Result<CommandOutput> {
        let mut process = Command::new(&command.program);
        process.args(&command.args).current_dir(&command.dir);

        if !self.path_prefix.is_empty() {
            let mut paths: Vec<PathBuf> = self.path_prefix.clone();
            let existing: OsString = std::env::var_os("PATH").unwrap_or_default();
            paths.extend(std::env::split_paths(&existing));
            process.env("PATH", std::env::join_paths(paths)?);
        }

        let output = process
            .output()
            .wrap_err_with(|| format!("Failed to start `{}`", command.program))?;
        Ok(CommandOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_runner_runs_in_the_directory_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessRunner::new();

        let ok = ExternalCommand::new(dir.path(), "sh", &["-c", "pwd; echo err >&2"], "pwd");
        let out = run_checked(&runner, &ok).unwrap();
        let printed = std::fs::canonicalize(out.stdout.trim()).unwrap();
        assert_eq!(printed, std::fs::canonicalize(dir.path()).unwrap());

        let bad = ExternalCommand::new(dir.path(), "sh", &["-c", "echo boom >&2; exit 3"], "fail");
        let err = run_checked(&runner, &bad).unwrap_err().to_string();
        assert!(err.contains("exit 3") && err.contains("boom"), "{err}");

        let missing = ExternalCommand::new(dir.path(), "definitely-not-a-program", &[], "x");
        assert!(run_checked(&runner, &missing).is_err());
    }

    #[test]
    fn from_command_keeps_program_args_and_directory() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("true").current_dir("/somewhere");
        let external = ExternalCommand::from_command(&command, Path::new("/fallback"), "t");
        assert_eq!(external.program, "sh");
        assert_eq!(external.args, ["-c", "true"]);
        assert_eq!(external.dir, Path::new("/somewhere"));
        assert_eq!(external.label, "t");
    }
}

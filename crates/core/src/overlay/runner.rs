//! Running external commands (composer, rebuild tasks) behind a trait, so tests can
//! substitute a fake while the real implementation runs the process in the repo directory.

use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
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

/// Where macOS keeps developer tools outside the skeletal `PATH` of a bundled launch
/// (Dock, Finder): Homebrew on both architectures, MacPorts, Docker Desktop's CLI
/// directory and its app bundle. Only consulted when a program resolves nowhere, so
/// normal resolution never changes.
pub fn tool_fallback_dirs(home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/local/bin"),
        PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"),
    ];
    if let Some(h) = home {
        dirs.push(h.join(".docker").join("bin"));
    }
    dirs
}

/// The `candidates` that exist and are not already listed in `path_var`.
pub fn missing_dirs(path_var: Option<&std::ffi::OsStr>, candidates: &[PathBuf]) -> Vec<PathBuf> {
    let listed: Vec<PathBuf> = path_var
        .map(|v| std::env::split_paths(v).collect())
        .unwrap_or_default();
    candidates
        .iter()
        .filter(|d| d.is_dir() && !listed.contains(d))
        .cloned()
        .collect()
}

/// Whether `program` (no directory part) is found nowhere in `path_var`.
fn program_missing(path_var: Option<&std::ffi::OsStr>, program: &str) -> bool {
    let listed: Vec<PathBuf> = path_var
        .map(std::env::split_paths)
        .map(Iterator::collect)
        .unwrap_or_default();
    !listed.iter().any(|dir| dir.join(program).is_file())
}

/// Runs the process for real, with the command's directory as its working directory.
#[derive(Debug, Clone, Default)]
pub struct ProcessRunner {
    /// Directories put in front of `PATH` for the child (tests use this for a stub `composer`).
    path_prefix: Vec<PathBuf>,
    /// Kill the child after this long. `None` (the default) waits forever, which the
    /// long-running callers (`composer update`, rebuild tasks) rely on.
    timeout: Option<Duration>,
}

/// The error a [`ProcessRunner`] with a timeout returns after killing a child that ran too
/// long. Downcast from the `eyre::Report` to tell it from a spawn failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedOut {
    pub program: String,
    pub after: Duration,
}

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` timed out after {:.1}s and was killed",
            self.program,
            self.after.as_secs_f32()
        )
    }
}

impl std::error::Error for TimedOut {}

fn drain<T: Read + Send + 'static>(stream: Option<T>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stream {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    })
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

    /// Kill (and reap) the child if it runs longer than `timeout`, failing with [`TimedOut`].
    /// Output is read concurrently, so a chatty child cannot block on a full pipe.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    fn run_with_timeout(
        &self,
        mut process: Command,
        program: &str,
        timeout: Duration,
    ) -> eyre::Result<CommandOutput> {
        let mut child = process
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .wrap_err_with(|| format!("Failed to start `{program}`"))?;
        let out = drain(child.stdout.take());
        let err = drain(child.stderr.take());
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child
                .try_wait()
                .wrap_err("Failed to wait for the process")?
            {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // A grandchild may still hold the pipes open; do not join the readers.
                    return Err(TimedOut {
                        program: program.to_string(),
                        after: timeout,
                    }
                    .into());
                }
                None => thread::sleep(Duration::from_millis(10)),
            }
        };
        Ok(CommandOutput {
            success: status.success(),
            code: status.code(),
            stdout: String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned(),
            stderr: String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned(),
        })
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&self, command: &ExternalCommand) -> eyre::Result<CommandOutput> {
        let mut process = Command::new(&command.program);
        process.args(&command.args).current_dir(&command.dir);

        let mut prefix = self.path_prefix.clone();
        if prefix.is_empty()
            && !command.program.contains(std::path::MAIN_SEPARATOR)
            && program_missing(std::env::var_os("PATH").as_deref(), &command.program)
        {
            // Nowhere on PATH (a Dock/Finder launch inherits a skeletal one): look in the
            // usual macOS tool locations before reporting the tool missing.
            let home = std::env::var_os("HOME").map(PathBuf::from);
            prefix = missing_dirs(
                std::env::var_os("PATH").as_deref(),
                &tool_fallback_dirs(home),
            );
        }
        if !prefix.is_empty() {
            let mut paths = prefix;
            let existing: OsString = std::env::var_os("PATH").unwrap_or_default();
            paths.extend(std::env::split_paths(&existing));
            process.env("PATH", std::env::join_paths(paths)?);
        }

        if let Some(timeout) = self.timeout {
            return self.run_with_timeout(process, &command.program, timeout);
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
    fn fallback_dirs_cover_homebrew_macports_and_docker() {
        let dirs = tool_fallback_dirs(None);
        assert_eq!(
            dirs,
            [
                "/opt/homebrew/bin",
                "/opt/homebrew/sbin",
                "/usr/local/bin",
                "/opt/local/bin",
                "/Applications/Docker.app/Contents/Resources/bin",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>()
        );
        let home = tempfile::tempdir().unwrap();
        let dirs = tool_fallback_dirs(Some(home.path().into()));
        assert_eq!(
            dirs.last().unwrap(),
            &home.path().join(".docker").join("bin")
        );
    }

    #[test]
    fn only_existing_unlisted_dirs_are_added() {
        let home = tempfile::tempdir().unwrap();
        let present = home.path().join("present");
        std::fs::create_dir(&present).unwrap();
        let candidates = vec![
            PathBuf::from("/definitely/not/here"),
            present.clone(),
            PathBuf::from("/usr/bin"),
        ];
        let path_var = std::env::join_paths([PathBuf::from("/usr/bin")]).unwrap();
        assert_eq!(
            missing_dirs(Some(path_var.as_os_str()), &candidates),
            [present]
        );
    }

    #[test]
    fn a_program_on_path_needs_no_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mytool"), "").unwrap();
        let path_var = std::env::join_paths([dir.path()]).unwrap();
        assert!(!program_missing(Some(path_var.as_os_str()), "mytool"));
        assert!(program_missing(Some(path_var.as_os_str()), "other-tool"));
        assert!(program_missing(None, "other-tool"));
    }

    fn pid_alive(pid: &str) -> bool {
        Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn timeout_kills_a_hung_child_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let script = format!("echo $$ > {}; exec sleep 30", pidfile.display());
        let runner = ProcessRunner::new().with_timeout(Duration::from_millis(200));
        let started = Instant::now();
        let err = runner
            .run(&ExternalCommand::new(
                dir.path(),
                "sh",
                &["-c", &script],
                "hang",
            ))
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(err.downcast_ref::<TimedOut>().is_some(), "{err:#}");
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        assert!(!pid_alive(pid.trim()), "child must be dead and reaped");
    }

    #[test]
    fn timeout_does_not_deadlock_on_a_chatty_child() {
        let dir = tempfile::tempdir().unwrap();
        // Far more than a pipe buffer on both streams, then hang.
        let script = "yes x | head -c 1000000; yes y | head -c 1000000 >&2; exec sleep 30";
        let runner = ProcessRunner::new().with_timeout(Duration::from_millis(500));
        let started = Instant::now();
        let err = runner
            .run(&ExternalCommand::new(
                dir.path(),
                "sh",
                &["-c", script],
                "chatty",
            ))
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(err.downcast_ref::<TimedOut>().is_some());
    }

    #[test]
    fn timeout_leaves_fast_and_chatty_successes_alone() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessRunner::new().with_timeout(Duration::from_secs(20));
        let out = runner
            .run(&ExternalCommand::new(
                dir.path(),
                "sh",
                &["-c", "yes x | head -c 500000; echo err >&2; exit 2"],
                "ok",
            ))
            .unwrap();
        assert_eq!(out.code, Some(2));
        assert_eq!(out.stdout.len(), 500000);
        assert_eq!(out.stderr.trim(), "err");
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

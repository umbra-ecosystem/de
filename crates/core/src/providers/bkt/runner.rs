//! Running the `bkt` binary. Kept behind a small `Send + Sync` trait so the adapter is
//! `Send` and tests can substitute canned output (the overlay `CommandRunner` is neither
//! `Send` nor stdin/timeout aware).

use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// What a finished process printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawOutput {
    /// `None` when killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl RawOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// The process could not be run at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The binary does not exist (spawn `NotFound`).
    NotFound,
    /// Any other spawn/wait/timeout failure.
    Failed(String),
}

/// Runs `bkt` with the given arguments (argv only; never a shell string).
pub trait BktRunner: Send + Sync {
    fn run(&self, args: &[String]) -> Result<RawOutput, RunError>;
}

/// The real thing: spawns `program` with closed stdin, captures output, and kills it after
/// `timeout` so a hung `bkt` (keychain prompt, dead network) can never hang `de`.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    program: String,
    timeout: Duration,
}

impl SystemRunner {
    pub fn new() -> Self {
        Self {
            program: "bkt".into(),
            timeout: Duration::from_secs(120),
        }
    }

    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::new()
    }
}

fn drain<T: Read + Send + 'static>(stream: Option<T>) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stream {
            let _ = s.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

impl BktRunner for SystemRunner {
    fn run(&self, args: &[String]) -> Result<RawOutput, RunError> {
        let mut child = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    RunError::NotFound
                } else {
                    RunError::Failed(format!("could not start `{}`: {e}", self.program))
                }
            })?;
        let out = drain(child.stdout.take());
        let err = drain(child.stderr.take());
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RunError::Failed(format!(
                        "`{}` timed out after {}s",
                        self.program,
                        self.timeout.as_secs()
                    )));
                }
                Ok(None) => thread::sleep(Duration::from_millis(15)),
                Err(e) => return Err(RunError::Failed(format!("waiting for bkt: {e}"))),
            }
        };
        Ok(RawOutput {
            code: status.code(),
            stdout: out.join().unwrap_or_default(),
            stderr: err.join().unwrap_or_default(),
        })
    }
}

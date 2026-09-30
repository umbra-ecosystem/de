//! Raw logs of sync runs: one file per run, named by its start time, pruned to a configurable count.
//!
//! A run is started with [`begin`] and ends when its [`RunLog`] guard drops. While it lives, [`log`] appends
//! a line to its file. With no run in progress [`log`] does nothing, so the provider adapters can call it
//! freely (a gateway write, for instance, is not logged here).
//!
//! The files hold what the tools printed, raw, so they can contain ticket titles and comments: they stay in
//! the local data directory and should be reviewed before being shared.

use std::fmt::Display;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eyre::Context;

use crate::config::Config;
use crate::utils::get_project_dirs;

/// Files kept when `[logs] keep` is not set.
pub const DEFAULT_KEEP: usize = 20;

/// The most of one stream (stdout or stderr) of one call written to a log.
pub const MAX_STREAM_BYTES: usize = 256 * 1024;

/// Tests that start a run take this, since the current run is process-wide.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

const PREFIX: &str = "sync-";
const SUFFIX: &str = ".log";

struct Inner {
    file: Mutex<File>,
    started: Instant,
}

static CURRENT: Mutex<Option<Arc<Inner>>> = Mutex::new(None);

/// A run in progress. Dropping it ends the run.
pub struct RunLog {
    inner: Arc<Inner>,
    path: PathBuf,
}

impl RunLog {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunLog {
    fn drop(&mut self) {
        write_line(&self.inner, "run ended");
        let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, &self.inner)) {
            *current = None;
        }
    }
}

/// Where logs live: `<data dir>/logs`.
pub fn default_dir() -> eyre::Result<PathBuf> {
    Ok(get_project_dirs()?.data_dir().join("logs"))
}

/// Starts a run in the default directory, keeping the number of files `config` asks for. A log that cannot
/// be written must never stop a sync, so a failure here is only reported to `tracing` and gives `None`.
pub fn begin_default(now: i64, config: &Config) -> Option<RunLog> {
    let dir = match default_dir() {
        Ok(dir) => dir,
        Err(e) => {
            tracing::warn!("sync log disabled: {e:#}");
            return None;
        }
    };
    match begin(&dir, now, config.log_keep()) {
        Ok(run) => Some(run),
        Err(e) => {
            tracing::warn!("sync log disabled: {e:#}");
            None
        }
    }
}

/// Starts a run: prunes old files so that at most `keep - 1` remain, creates `sync-<UTC time>.log` and makes it
/// the current run.
pub fn begin(dir: &Path, now: i64, keep: usize) -> eyre::Result<RunLog> {
    fs::create_dir_all(dir)
        .wrap_err_with(|| format!("Failed to create the log directory {}", dir.display()))?;
    prune(dir, keep.max(1) - 1)?;

    let stamp = utc_stamp(now);
    let mut n = 0;
    let (file, path) = loop {
        let name = if n == 0 {
            format!("{PREFIX}{stamp}{SUFFIX}")
        } else {
            format!("{PREFIX}{stamp}-{n}{SUFFIX}")
        };
        let path = dir.join(name);
        match File::options().write(true).create_new(true).open(&path) {
            Ok(file) => break (file, path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => {
                return Err(e)
                    .wrap_err_with(|| format!("Failed to create the log {}", path.display()));
            }
        }
    };

    let inner = Arc::new(Inner {
        file: Mutex::new(file),
        started: Instant::now(),
    });
    write_line(&inner, &format!("run started {}", utc_stamp(now)));
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(inner.clone());
    Ok(RunLog { inner, path })
}

/// Appends a line to the current run's log, if there is one. Each line starts with the seconds since the run began.
pub fn log(line: impl Display) {
    let current = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(inner) = current {
        write_line(&inner, &line.to_string());
    }
}

/// Appends raw tool output to the current run's log, indented and capped at [`MAX_STREAM_BYTES`].
pub fn log_block(title: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    let (shown, cut) = cap(text, MAX_STREAM_BYTES);
    let mut block = format!("{title} ({} bytes)", text.len());
    for line in shown.lines() {
        block.push_str("\n    | ");
        block.push_str(line);
    }
    if cut {
        block.push_str(&format!("\n    | ... cut at {MAX_STREAM_BYTES} bytes"));
    }
    log(block);
}

fn cap(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn write_line(inner: &Inner, line: &str) {
    let elapsed = inner.started.elapsed().as_secs_f64();
    if let Ok(mut file) = inner.file.lock() {
        // A full disk must not break the sync it describes.
        let _ = writeln!(file, "[+{elapsed:8.3}s] {line}");
    }
}

/// Unix seconds of `YYYYMMDD-HHMMSS` (UTC), or `None` for anything else.
fn parse_stamp(d: &str, t: &str) -> Option<i64> {
    if d.len() != 8 || t.len() != 6 || !d.chars().chain(t.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (y, m, day): (i64, i64, i64) = (d[..4].parse().ok()?, d[4..6].parse().ok()?, d[6..].parse().ok()?);
    let (hh, mm, ss): (i64, i64, i64) = (t[..2].parse().ok()?, t[2..4].parse().ok()?, t[4..].parse().ok()?);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// One log file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    /// The file name, which is also its id for [`read`].
    pub name: String,
    pub bytes: u64,
}

impl LogFile {
    /// When the run started, in unix seconds, read back from the name.
    pub fn started_unix(&self) -> Option<i64> {
        let stem = self.name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
        let mut parts = stem.split('-');
        parse_stamp(parts.next()?, parts.next()?)
    }

    /// `2026-10-01 09:40:12` (UTC), read back from the name.
    pub fn started(&self) -> String {
        let stem = self
            .name
            .strip_prefix(PREFIX)
            .and_then(|s| s.strip_suffix(SUFFIX))
            .unwrap_or(&self.name);
        // `YYYYMMDD-HHMMSS[-n]`
        let digits: Vec<&str> = stem.split('-').collect();
        match digits.as_slice() {
            [d, t, ..] if d.len() == 8 && t.len() == 6 => format!(
                "{}-{}-{} {}:{}:{}",
                &d[..4],
                &d[4..6],
                &d[6..],
                &t[..2],
                &t[2..4],
                &t[4..]
            ),
            _ => stem.to_string(),
        }
    }
}

/// The log files in `dir`, newest first. A missing directory is an empty list.
pub fn list(dir: &Path) -> eyre::Result<Vec<LogFile>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(e).wrap_err_with(|| format!("Failed to read {}", dir.display()));
        }
    };
    let mut files: Vec<LogFile> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            is_log_name(&name).then(|| LogFile {
                bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                name,
            })
        })
        .collect();
    // Names sort by time (fixed-width stamp); a `-n` suffix keeps later runs of one second after earlier ones.
    files.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(files)
}

/// The text of one log. Only names that [`list`] would return are accepted (no paths).
pub fn read(dir: &Path, name: &str) -> eyre::Result<String> {
    if !is_log_name(name) {
        eyre::bail!("{name} is not a sync log");
    }
    let path = dir.join(name);
    let bytes = fs::read(&path).wrap_err_with(|| format!("Failed to read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn is_log_name(name: &str) -> bool {
    name.starts_with(PREFIX)
        && name.ends_with(SUFFIX)
        && !name.contains(['/', '\\'])
        && !name.contains("..")
}

/// Deletes the oldest logs so that at most `keep` remain.
pub fn prune(dir: &Path, keep: usize) -> eyre::Result<usize> {
    let files = list(dir)?;
    let mut removed = 0;
    for old in files.into_iter().skip(keep) {
        if fs::remove_file(dir.join(&old.name)).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// `20261001-094012` (UTC) from unix seconds.
fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_are_utc_and_fixed_width() {
        assert_eq!(utc_stamp(0), "19700101-000000");
        assert_eq!(utc_stamp(1_700_000_000), "20231114-221320");
    }

    // One test owns the process-wide "current run", so tests cannot interleave.
    #[test]
    fn a_run_writes_its_own_file_and_old_runs_are_pruned() {
        let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        log("nobody is listening"); // no run: a no-op, not a panic

        for i in 0..5 {
            let run = begin(dir.path(), 1_700_000_000 + i * 10, 3).unwrap();
            log(format!("step {i}"));
            log_block("stdout", "line one\nline two");
            drop(run);
        }

        let files = list(dir.path()).unwrap();
        assert_eq!(files.len(), 3, "keeps the newest 3: {files:?}");
        assert_eq!(files[0].started(), "2023-11-14 22:14:00");
        let newest = read(dir.path(), &files[0].name).unwrap();
        assert!(newest.contains("run started 20231114-221400"), "{newest}");
        assert!(newest.contains("step 4"), "{newest}");
        assert!(newest.contains("    | line two"), "{newest}");
        assert!(newest.contains("run ended"), "{newest}");
        assert!(!newest.contains("step 3"), "each run has its own file");

        // Two runs in one second do not overwrite each other.
        let a = begin(dir.path(), 1_800_000_000, 10).unwrap();
        let b = begin(dir.path(), 1_800_000_000, 10).unwrap();
        assert_ne!(a.path(), b.path());
        drop(a);
        drop(b);
        log("after"); // the guards cleared the current run
    }

    #[test]
    fn a_name_reads_back_as_the_unix_time_it_was_made_from() {
        for secs in [0, 1_700_000_000, 1_790_000_123, 951_782_400] {
            let file = LogFile {
                name: format!("{PREFIX}{}{SUFFIX}", utc_stamp(secs)),
                bytes: 0,
            };
            assert_eq!(file.started_unix(), Some(secs));
        }
        let twin = LogFile { name: "sync-20231114-221320-2.log".into(), bytes: 0 };
        assert_eq!(twin.started_unix(), Some(1_700_000_000));
        assert_eq!(LogFile { name: "sync-x.log".into(), bytes: 0 }.started_unix(), None);
    }

    #[test]
    fn read_refuses_anything_but_a_log_name() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path(), "../config.toml").is_err());
        assert!(read(dir.path(), "sync-../../x.log").is_err());
        assert!(list(&dir.path().join("missing")).unwrap().is_empty());
    }

    #[test]
    fn long_output_is_cut_on_a_character_boundary() {
        let text = "é".repeat(MAX_STREAM_BYTES);
        let (shown, cut) = cap(&text, MAX_STREAM_BYTES);
        assert!(cut && shown.len() <= MAX_STREAM_BYTES);
    }
}

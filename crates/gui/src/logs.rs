//! The sync logs as the widgets show them: the files of [`de_core::synclog`] as list rows and text.

use std::path::Path;

use de_core::synclog::{self, LogFile};
use de_widgets::vm::LogRunVm;

/// The runs on disk, newest first, and the line under the list heading.
pub fn runs_in(dir: &Path, keep: usize) -> (Vec<LogRunVm>, String) {
    let runs = synclog::list(dir)
        .unwrap_or_default()
        .iter()
        .map(row)
        .collect();
    let note = format!(
        "Keeping the latest {keep} run{}. Change `keep` under [logs] in config.toml.",
        if keep == 1 { "" } else { "s" }
    );
    (runs, note)
}

/// The text of one log. A log that has since been pruned reads as a sentence that says what to do.
pub fn text_in(dir: &Path, id: &str) -> String {
    match synclog::read(dir, id) {
        Ok(text) => text,
        Err(_) => "This log is no longer there. Older logs are removed to keep the latest few; \
                   pick another run from the list."
            .to_string(),
    }
}

fn row(file: &LogFile) -> LogRunVm {
    LogRunVm {
        id: file.name.clone(),
        when: file.started(),
        size: size(file.bytes),
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", b / 1024),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_newest_first_with_readable_times_and_sizes() {
        let dir = tempfile::tempdir().unwrap();
        for now in [1_700_000_000, 1_700_000_100] {
            let run = synclog::begin(dir.path(), now, 10).unwrap();
            synclog::log("hello");
            drop(run);
        }
        let (runs, note) = runs_in(dir.path(), 10);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].when, "2023-11-14 22:15:00");
        assert_eq!(runs[1].when, "2023-11-14 22:13:20");
        assert!(runs[0].size.ends_with(" B"), "{}", runs[0].size);
        assert!(note.contains("latest 10 runs"), "{note}");
        assert!(text_in(dir.path(), &runs[0].id).contains("hello"));
    }

    #[test]
    fn a_missing_log_reads_as_guidance_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(text_in(dir.path(), "sync-20200101-000000.log").contains("pick another run"));
        assert!(runs_in(&dir.path().join("none"), 1).0.is_empty());
        assert_eq!(size(1536), "1 KB");
        assert_eq!(size(3 * 1024 * 1024), "3.0 MB");
    }
}

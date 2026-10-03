//! `de-gui`: the desktop app. The `de-widgets` views over the real engine.
//!
//! - [`tickets`]: the Jira mirror and local tracking, read as widget tickets (pure given the two databases).
//! - [`logs`]: the raw sync logs (one file per run, see [`de_core::synclog`]) as list rows and text.
//! - [`sync`]: a read-only Jira sync through the engine (`acli`), reported in the terms the status bar shows.
//! - [`lifecycle`]: opening, closing and switching a workspace as real steps on worker threads.
//! - [`measure`]: the open workspace's services, git state and active ticket, read off the UI thread.
//! - [`store::CoreStore`]: the [`de_widgets::Store`] that serves those tickets, runs that sync and
//!   opens and closes workspaces. What the engine does not serve to the GUI yet (pull requests,
//!   review, shipping) is still the widgets' simulation, running on the real tickets.

// The widgets crate allows these for its view models; the store implements traits over them.
#![allow(clippy::result_large_err)]

pub mod audit;
pub mod lifecycle;
pub mod localtime;
pub mod logs;
pub mod mapping;
pub mod measure;
pub mod schedule;
pub mod store;
pub mod sync;
pub mod tickets;

use de_widgets::Session;

/// Finder/Dock launches inherit a skeletal PATH without Homebrew or Docker Desktop, so
/// `docker compose`, health checks and tasks would fail to find their tools (the runner
/// already covers the CLIs it spawns itself). Repair it once, before anything runs: only
/// directories that exist and are missing are appended, so a real shell PATH keeps its
/// order and a skeletal one gains the tools.
fn ensure_tool_path() {
    use de_core::overlay::{missing_dirs, tool_fallback_dirs};
    let path_var = std::env::var_os("PATH");
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut paths: Vec<std::path::PathBuf> =
        std::env::split_paths(&path_var.clone().unwrap_or_default()).collect();
    paths.extend(missing_dirs(path_var.as_deref(), &tool_fallback_dirs(home)));
    if let Ok(joined) = std::env::join_paths(&paths) {
        // Safe: this runs once on the main thread before the app spawns anything.
        unsafe {
            std::env::set_var("PATH", joined);
        }
    }
}

/// Open the app window over the real engine and run until it closes.
pub fn run() {
    ensure_tool_path();
    de_widgets::ui::run_with(|| Session::new(Box::new(store::CoreStore::open())));
}

//! `de-gui`: the desktop app. The `de-widgets` views over the real engine.
//!
//! - [`tickets`]: the Jira mirror and local tracking, read as widget tickets (pure given the two databases).
//! - [`logs`]: the raw sync logs (one file per run, see [`de_core::synclog`]) as list rows and text.
//! - [`sync`]: a read-only Jira sync through the engine (`acli`), reported in the terms the status bar shows.
//! - [`store::CoreStore`]: the [`de_widgets::Store`] that serves those tickets and runs that sync. What the
//!   engine does not serve to the GUI yet (pull requests, review, shipping, the workspace) is still the
//!   widgets' simulation, running on the real tickets.

// The widgets crate allows these for its view models; the store implements traits over them.
#![allow(clippy::result_large_err)]

pub mod audit;
pub mod localtime;
pub mod logs;
pub mod mapping;
pub mod schedule;
pub mod store;
pub mod sync;
pub mod tickets;

use de_widgets::Session;

/// Open the app window over the real engine and run until it closes.
pub fn run() {
    de_widgets::ui::run_with(|| Session::new(Box::new(store::CoreStore::open())));
}

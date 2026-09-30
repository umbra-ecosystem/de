//! View models: everything a view may read, and everything it may ask for.
//!
//! Nothing here depends on GPUI or on an engine. A screen is drawn from exactly one of these values.

pub mod app;
pub mod common;
pub mod intent;
pub mod screens;

pub use app::*;
pub use common::*;
pub use intent::*;
pub use screens::*;

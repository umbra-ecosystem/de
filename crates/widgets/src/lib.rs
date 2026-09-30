//! `de-widgets`: the view layer of the de GUI, separated from state.
//!
//! - [`vm`]: view models. Plain data describing exactly what a screen shows and what it may ask for.
//! - [`store::Store`]: the seam to whatever supplies the data. The showcase uses [`sim::Sim`]; `de-app` will use the engine.
//! - [`session::Session`]: UI state (routes, tabs, sheets, toasts, drafts) on top of a store.
//! - [`ui`]: the GPUI widgets and screens. They draw view models and emit [`vm::Intent`]s, nothing else.
//!
//! This crate never touches the engine, the data or SQLite. New UI is built and shown in the `showcase` binary first.

// View models are plain data rebuilt per frame; boxing their variants would only add noise.
#![allow(
    clippy::large_enum_variant,
    clippy::result_large_err,
    clippy::too_many_arguments
)]

pub mod session;
pub mod sim;
pub mod store;
pub mod ui;
pub mod vm;

pub use session::Session;
pub use store::{Outcome, ReviewSel, Store};

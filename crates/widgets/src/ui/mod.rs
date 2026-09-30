//! GPUI widgets and screens. They draw view models and emit intents; they hold no state of their own
//! except the text inputs owned by [`app::AppView`].

pub mod app;
pub mod assets;
pub mod ctx;
pub mod filter_menu;
pub mod panels;
pub mod screens;
pub mod shell;
pub mod theme;
pub mod widgets;

pub use app::{AppView, run};

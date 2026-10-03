//! Core of `de`: workspaces, projects, tasks and Docker Compose orchestration.

pub mod activation;
pub mod config;
pub mod constants;
pub mod domain;
pub mod gateway;
pub mod git;
pub mod integration;
pub mod next;
pub mod overlay;
pub mod project;
pub mod providers;
pub mod store;
pub mod switch;
pub mod synclog;
pub mod sync;
pub mod types;
pub mod utils;
pub mod workspace;

#[cfg(test)]
mod testsupport;

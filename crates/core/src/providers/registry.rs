//! Builds provider adapters from configuration.
//!
//! # Where an adapter author plugs in
//!
//! Each adapter needs exactly one constructor per trait, and each has a clearly marked
//! `build_*` function below whose body currently returns "adapter not available":
//!
//! | Function | Replace the body with | File to add the adapter in |
//! |---|---|---|
//! | [`build_ticket_provider`] | `Ok(Box::new(acli::AcliJira::new(config)?))` | `providers/acli.rs` |
//! | [`build_ticket_writer`] | `Ok(Box::new(acli::AcliJiraWriter::new(config)?))` | `providers/acli.rs` |
//! | [`build_code_host`] | `Ok(Box::new(bkt::BktHost::new(config)?))` | `providers/bkt.rs` |
//! | [`build_code_host_writer`] | `Ok(Box::new(bkt::BktHostWriter::new(config)?))` | `providers/bkt.rs` |
//!
//! Nothing else needs to change: [`Providers::from_config`], `de sync` and
//! `de providers check` all go through these functions. Construction must not spawn
//! processes or touch the network (that is what `health` is for); report a missing binary
//! from the first call instead, as [`ProviderError::NotInstalled`].
//!
//! The write constructors are for the write gateway only; sync never calls them.

use super::error::{ProviderError, ProviderResult};
use super::traits::{CodeHost, CodeHostWriter, TicketProvider, TicketWriter};
use crate::config::Config;

/// The read-only providers built from a configuration.
///
/// Each side is a `Result` so one adapter being unavailable (not installed, not written yet,
/// misconfigured) never prevents using the other.
pub struct Providers {
    pub jira: ProviderResult<Box<dyn TicketProvider>>,
    pub code_host: ProviderResult<Box<dyn CodeHost>>,
}

impl Providers {
    pub fn from_config(config: &Config) -> Self {
        Self {
            jira: build_ticket_provider(config),
            code_host: build_code_host(config),
        }
    }
}

fn adapter_unavailable(tool: &str) -> ProviderError {
    ProviderError::not_installed(tool, "adapter not available in this build")
}

/// The Jira reader. **Plug-in point for the `acli` adapter (read).**
pub fn build_ticket_provider(_config: &Config) -> ProviderResult<Box<dyn TicketProvider>> {
    Err(adapter_unavailable("acli"))
}

/// The Jira writer, for the write gateway only. **Plug-in point for the `acli` adapter (write).**
pub fn build_ticket_writer(_config: &Config) -> ProviderResult<Box<dyn TicketWriter>> {
    Err(adapter_unavailable("acli"))
}

/// The Bitbucket reader. **Plug-in point for the `bkt` adapter (read).**
pub fn build_code_host(config: &Config) -> ProviderResult<Box<dyn CodeHost>> {
    Ok(Box::new(super::bkt::BktHost::new(config)?))
}

/// The Bitbucket writer, for the write gateway only. **Plug-in point for the `bkt` adapter (write).**
pub fn build_code_host_writer(config: &Config) -> ProviderResult<Box<dyn CodeHostWriter>> {
    Ok(Box::new(super::bkt::BktHostWriter::new(config)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderErrorKind;

    #[test]
    fn every_constructor_reports_an_unavailable_adapter_until_filled_in() {
        let config = Config::default();
        let providers = Providers::from_config(&config);
        assert_eq!(
            providers.jira.err().map(|e| e.kind()),
            Some(ProviderErrorKind::NotInstalled)
        );
        // The bkt adapter is built without spawning anything; a missing binary shows up in `health`.
        assert!(providers.code_host.is_ok());
        assert!(build_code_host_writer(&config).is_ok());
        assert!(build_ticket_writer(&config).is_err());
    }
}

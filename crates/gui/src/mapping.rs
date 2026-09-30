//! The Jira mapping on the settings page, read from and written to the `[jira]` section of `config.toml`.
//!
//! Both directions are pure over a [`Config`] so they can be tested without touching the disk. An empty text
//! unsets a key, and a section that ends up empty is dropped, so the file only holds what the user chose.

use de_core::config::{Config, JiraConfig, JiraStatuses};
use de_widgets::vm::{MappingKey, MappingRow};

/// Every editable row with its stored value (empty when unset).
pub fn rows(config: &Config) -> Vec<MappingRow> {
    MappingKey::LIST
        .iter()
        .map(|key| MappingRow {
            key: *key,
            value: get(config, *key),
        })
        .collect()
}

pub fn get(config: &Config, key: MappingKey) -> String {
    let jira = config.jira();
    let statuses = jira.statuses.unwrap_or_default();
    match key {
        MappingKey::ReviewStatus => statuses.review.unwrap_or_default(),
        MappingKey::AlphaStatus => statuses.alpha_testing.unwrap_or_default(),
        MappingKey::UatStatus => statuses.uat.unwrap_or_default(),
        MappingKey::ReturnedStatus => statuses.returned.unwrap_or_default(),
        MappingKey::DoneStatuses => statuses.done.join(", "),
        MappingKey::SignedOffStatuses => statuses.signed_off.join(", "),
        MappingKey::ReviewJql => jira.review_jql.unwrap_or_default(),
        MappingKey::AccountId => jira.account_id.unwrap_or_default(),
    }
}

/// Sets `key` to `text` (trimmed; empty unsets it).
pub fn set(config: &mut Config, key: MappingKey, text: &str) {
    let text = text.trim();
    let one = || (!text.is_empty()).then(|| text.to_string());
    let list = || -> Vec<String> {
        text.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    };
    let mut jira = config.jira();
    let mut statuses = jira.statuses.take().unwrap_or_default();
    match key {
        MappingKey::ReviewStatus => statuses.review = one(),
        MappingKey::AlphaStatus => statuses.alpha_testing = one(),
        MappingKey::UatStatus => statuses.uat = one(),
        MappingKey::ReturnedStatus => statuses.returned = one(),
        MappingKey::DoneStatuses => statuses.done = list(),
        MappingKey::SignedOffStatuses => statuses.signed_off = list(),
        MappingKey::ReviewJql => jira.review_jql = one(),
        MappingKey::AccountId => jira.account_id = one(),
    }
    jira.statuses = (statuses != JiraStatuses::default()).then_some(statuses);
    config.jira = (jira != JiraConfig::default()).then_some(jira);
}

/// Saves these settings to the config file in one write, returning the config as saved.
pub fn save_all(changes: &[(MappingKey, String)]) -> eyre::Result<Config> {
    Config::mutate_persisted(|config| {
        for (key, text) in changes {
            set(config, *key, text);
        }
    })
}

/// Clears every mapping setting, returning the config as saved.
pub fn reset() -> eyre::Result<Config> {
    Config::mutate_persisted(|config| {
        for key in MappingKey::LIST {
            set(config, key, "");
        }
    })
}

/// What saving these takes effect on, for the message after saving.
pub fn effect(keys: impl IntoIterator<Item = MappingKey>) -> &'static str {
    let mut later = false;
    let mut now = false;
    for key in keys {
        match key {
            MappingKey::ReviewStatus | MappingKey::ReviewJql | MappingKey::AccountId => later = true,
            _ => now = true,
        }
    }
    match (now, later) {
        (_, false) => "Applied to the ticket list now.",
        (false, true) => "Used from the next sync.",
        (true, true) => "Status names apply now; the review query, review status and account id from the next sync.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_config_reads_as_all_unset() {
        assert!(rows(&Config::default()).iter().all(|r| r.value.is_empty()));
    }

    #[test]
    fn values_round_trip_and_lists_are_comma_separated() {
        let mut c = Config::default();
        set(&mut c, MappingKey::ReviewStatus, "  Code Review ");
        set(&mut c, MappingKey::DoneStatuses, "Done, Closed ,, Shipped");
        set(&mut c, MappingKey::ReviewJql, "project = PROJ");
        assert_eq!(get(&c, MappingKey::ReviewStatus), "Code Review");
        assert_eq!(get(&c, MappingKey::DoneStatuses), "Done, Closed, Shipped");
        assert_eq!(c.jira_statuses().review_name(), "Code Review");
        assert_eq!(c.jira_statuses().done_names(), ["Done", "Closed", "Shipped"]);
        assert_eq!(c.review_jql(), "project = PROJ");
    }

    #[test]
    fn clearing_the_last_value_removes_the_section_from_the_file() {
        let mut c = Config::default();
        set(&mut c, MappingKey::AlphaStatus, "Testing");
        set(&mut c, MappingKey::AccountId, "abc");
        set(&mut c, MappingKey::AlphaStatus, "");
        assert!(c.jira.is_some());
        assert!(c.jira.as_ref().unwrap().statuses.is_none());
        set(&mut c, MappingKey::AccountId, "  ");
        assert!(c.jira.is_none(), "an empty [jira] is not written");
    }

    #[test]
    fn effect_says_when_a_change_applies() {
        use MappingKey::*;
        assert_eq!(effect([UatStatus]), "Applied to the ticket list now.");
        assert_eq!(effect([ReviewJql]), "Used from the next sync.");
        assert!(effect([UatStatus, AccountId]).starts_with("Status names apply now"));
    }

    #[test]
    fn it_survives_the_toml_round_trip_other_settings_untouched() {
        let mut c = Config::parse("[jira]\nsite = \"acme.atlassian.net\"\n").unwrap();
        set(&mut c, MappingKey::UatStatus, "User Acceptance");
        let text = toml::to_string_pretty(&c).unwrap();
        let back = Config::parse(&text).unwrap();
        assert_eq!(get(&back, MappingKey::UatStatus), "User Acceptance");
        assert_eq!(back.jira().site.as_deref(), Some("acme.atlassian.net"));
    }
}

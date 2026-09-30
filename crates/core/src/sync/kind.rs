//! Deriving a ticket's kind (normal or hotfix) from its synced PRs.
//!
//! A ticket is a **hotfix** when any of its *open* PRs targets that repo's production
//! branch (the repo's configured `[branches] production`, else `master` or `main`), unless
//! the author set a manual `kind_override`. `uat` and `develop` are never production.

use std::fmt;

use super::hosted::{HostedRepo, is_production_branch};
use crate::domain::{TicketKey, TicketKind};
use crate::providers::{Pr, PrState};
use crate::store::{Store, prs, tickets};

/// Why a ticket has the kind it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindReason {
    /// The author set it by hand.
    Override,
    /// An open PR targets the repo's production branch.
    ProductionPr {
        repo: String,
        pr: u64,
        destination: String,
    },
    /// No open PR targets a production branch (or there are no PRs yet).
    NoProductionPr,
}

impl fmt::Display for KindReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KindReason::Override => f.write_str("set by hand"),
            KindReason::ProductionPr {
                repo,
                pr,
                destination,
            } => write!(f, "{repo} PR #{pr} targets {destination}"),
            KindReason::NoProductionPr => f.write_str("no open PR targets a production branch"),
        }
    }
}

/// A ticket kind and the reason for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedKind {
    pub kind: TicketKind,
    pub reason: KindReason,
}

/// Pure derivation. `prs` are the ticket's PRs in any state (only open ones count);
/// `hosted` supplies each repo's production branch (repos not listed use `master`/`main`).
pub fn derive_kind(
    kind_override: Option<TicketKind>,
    prs: &[Pr],
    hosted: &[HostedRepo],
) -> DerivedKind {
    if let Some(kind) = kind_override {
        return DerivedKind {
            kind,
            reason: KindReason::Override,
        };
    }

    let hotfix_pr = prs.iter().filter(|p| p.state == PrState::Open).find(|p| {
        let branches = HostedRepo::find(hosted, &p.repo).map(|h| &h.branches);
        is_production_branch(branches, &p.destination_branch)
    });

    match hotfix_pr {
        Some(p) => DerivedKind {
            kind: TicketKind::Hotfix,
            reason: KindReason::ProductionPr {
                repo: p.repo.clone(),
                pr: p.id,
                destination: p.destination_branch.clone(),
            },
        },
        None => DerivedKind {
            kind: TicketKind::Normal,
            reason: KindReason::NoProductionPr,
        },
    }
}

/// The derived kind of a ticket, reading its override from `state` and its PRs from `cache`.
///
/// This is the sibling read to [`crate::store::jira_cache::view`]: callers that show a
/// ticket's kind use it instead of `kind_override.unwrap_or(Normal)`.
pub fn ticket_kind(
    state: &Store,
    cache: &Store,
    key: &TicketKey,
    hosted: &[HostedRepo],
) -> eyre::Result<DerivedKind> {
    let kind_override = tickets::get(state, key)?.and_then(|t| t.kind_override);
    let prs = prs::for_ticket(cache, key)?;
    Ok(derive_kind(kind_override, &prs, hosted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::config::BranchesConfig;
    use crate::providers::fake::build::{key, pr};
    use crate::store::Kind;

    fn hosted(repo: &str, production: Option<&str>) -> HostedRepo {
        HostedRepo {
            project: "p".into(),
            repo: repo.into(),
            dir: "/x".into(),
            branches: BranchesConfig {
                production: production.map(String::from),
                ..Default::default()
            },
            deploy_environment: None,
        }
    }

    type Row<'a> = (&'a [HostedRepo], Vec<Pr>, Option<TicketKind>, TicketKind);

    fn kind_of(over: Option<TicketKind>, prs: &[Pr], hosted: &[HostedRepo]) -> TicketKind {
        derive_kind(over, prs, hosted).kind
    }

    #[test]
    fn derivation_table() {
        let unconfigured = [hosted("acme/web", None)];
        let main_repo = [hosted("acme/web", Some("main"))];
        let master_repo = [hosted("acme/web", Some("master"))];
        let to = |dest: &str| vec![pr("acme/web", 1, "feature/PROJ-1", dest)];

        use TicketKind::*;
        // (hosted config, PRs, override, expected)
        let rows: Vec<Row> = vec![
            // Normal flow: develop.
            (&unconfigured, to("develop"), None, Normal),
            // Unconfigured repos accept master or main.
            (&unconfigured, to("master"), None, Hotfix),
            (&unconfigured, to("main"), None, Hotfix),
            // A configured production branch is the only one that counts.
            (&main_repo, to("main"), None, Hotfix),
            (&main_repo, to("master"), None, Normal),
            (&master_repo, to("master"), None, Hotfix),
            (&master_repo, to("main"), None, Normal),
            // uat and other branches never make a hotfix.
            (&unconfigured, to("uat"), None, Normal),
            (&unconfigured, to("release/1.2"), None, Normal),
            // No PRs at all.
            (&unconfigured, vec![], None, Normal),
            // The override wins in both directions.
            (&unconfigured, to("develop"), Some(Hotfix), Hotfix),
            (&unconfigured, to("master"), Some(Normal), Normal),
            // A repo we know nothing about falls back to master/main.
            (&[], to("master"), None, Hotfix),
            (&[], to("develop"), None, Normal),
        ];
        for (i, (h, prs, over, expected)) in rows.iter().enumerate() {
            assert_eq!(kind_of(*over, prs, h), *expected, "row {i}");
        }
    }

    #[test]
    fn only_open_prs_count_and_any_pr_is_enough() {
        let mut merged = pr("acme/web", 1, "PROJ-1", "master");
        merged.state = PrState::Merged;
        let mut declined = pr("acme/web", 2, "PROJ-1", "main");
        declined.state = PrState::Declined;
        assert_eq!(
            kind_of(None, &[merged.clone(), declined], &[]),
            TicketKind::Normal
        );

        // One open production PR among several repos makes the ticket a hotfix.
        let open_prod = pr("acme/api", 3, "PROJ-1", "master");
        let d = derive_kind(
            None,
            &[pr("acme/web", 4, "PROJ-1", "develop"), open_prod],
            &[],
        );
        assert_eq!(d.kind, TicketKind::Hotfix);
        assert_eq!(
            d.reason,
            KindReason::ProductionPr {
                repo: "acme/api".into(),
                pr: 3,
                destination: "master".into()
            }
        );
        assert_eq!(d.reason.to_string(), "acme/api PR #3 targets master");
    }

    #[test]
    fn the_reason_names_the_override() {
        let d = derive_kind(Some(TicketKind::Hotfix), &[], &[]);
        assert_eq!(d.reason, KindReason::Override);
    }

    #[test]
    fn ticket_kind_reads_the_override_and_the_cached_prs() {
        let state = Store::open_in_memory(Kind::State).unwrap();
        let cache = Store::open_in_memory(Kind::Cache).unwrap();
        let k = key("PROJ-1");

        // Untracked ticket with a production PR in the cache: derived hotfix.
        prs::upsert(&cache, &pr("acme/web", 1, "feature/PROJ-1", "main"), 1).unwrap();
        // A different ticket's PR does not leak in (PROJ-1 vs PROJ-12).
        prs::upsert(&cache, &pr("acme/web", 2, "feature/PROJ-12", "develop"), 1).unwrap();
        let d = ticket_kind(&state, &cache, &k, &[]).unwrap();
        assert_eq!(d.kind, TicketKind::Hotfix);
        assert_eq!(
            ticket_kind(&state, &cache, &key("PROJ-12"), &[])
                .unwrap()
                .kind,
            TicketKind::Normal
        );

        // A manual override on the tracked ticket wins.
        tickets::claim(&state, &k, 1).unwrap();
        tickets::set_kind_override(&state, &k, Some(TicketKind::Normal), 2).unwrap();
        let d = ticket_kind(&state, &cache, &k, &[]).unwrap();
        assert_eq!(
            (d.kind, d.reason),
            (TicketKind::Normal, KindReason::Override)
        );
    }
}

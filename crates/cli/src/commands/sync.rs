//! `de sync` and `de providers check`.
//!
//! Each command gathers its inputs (impure) and hands them to a function that does the work
//! ([`run_sync`]) and to pure `render_*` functions, which are what the unit tests cover.

use de_core::{
    activation::WorkspaceRepo,
    config::Config,
    providers::{Health, ProviderError, Providers},
    store::{Kind, Store},
    sync::{
        DEFAULT_MIN_INTERVAL, HostedRepo, SourceOutcome, SourceReport, SyncContext, SyncReport,
        SyncSource, sync_all, uat_commits,
    },
};
use eyre::{Context, eyre};

use super::ticket::format_timestamp;
use crate::{utils::ui::UserInterface, workspace::Workspace};

fn now() -> eyre::Result<i64> {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .wrap_err("The system clock is before 1970")?;
    Ok(since_epoch.as_secs() as i64)
}

// ---------------------------------------------------------------- de sync

/// Runs a sync against already-built providers. Read-only towards the remotes.
pub fn run_sync(
    ctx: &SyncContext<'_>,
    providers: &Providers,
    repos: &[HostedRepo],
    only: Option<SyncSource>,
) -> eyre::Result<SyncReport> {
    let commits =
        uat_commits(ctx.state, repos).wrap_err("Failed to read the recorded uat merges")?;
    Ok(sync_all(
        ctx,
        providers.jira.as_deref(),
        providers.code_host.as_deref(),
        repos,
        &commits,
        only,
    ))
}

/// `de sync [--only jira|bitbucket] [--force]`.
pub fn sync(only: Option<SyncSource>, force: bool) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let config = Config::load()?;
    let state = Store::open_default(Kind::State)?;
    let cache = Store::open_default(Kind::Cache)?;

    // Repos come from the active workspace; without one only Jira can sync.
    let repos = match Workspace::active()? {
        Some(workspace) => {
            let repos = WorkspaceRepo::from_workspace(&workspace)
                .wrap_err("Failed to load the projects of the active workspace")?;
            HostedRepo::from_workspace_repos(&repos, &config)
        }
        None => Vec::new(),
    };

    let providers = Providers::from_config(&config);
    let now = now()?;
    let ctx = SyncContext {
        state: &state,
        cache: &cache,
        config: &config,
        now,
        force,
        min_interval: DEFAULT_MIN_INTERVAL,
    };
    let report = run_sync(&ctx, &providers, &repos, only)?;

    for line in render_sync_report(&report) {
        ui.writeln(&line)?;
    }
    Ok(())
}

/// Human-readable lines for a sync report: one per source, with indented details.
pub fn render_sync_report(report: &SyncReport) -> Vec<String> {
    if report.sources.is_empty() {
        return vec!["Nothing to sync.".into()];
    }
    let width = report
        .sources
        .iter()
        .map(|s| s.source.len())
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    for source in &report.sources {
        let (head, details) = describe_source(source);
        lines.push(format!("{:<width$}  {head}", source.source));
        for note in &source.notes {
            lines.push(format!("{:<width$}    note: {note}", ""));
        }
        for detail in details {
            lines.push(format!("{:<width$}    {detail}", ""));
        }
    }

    let bad = report.failures().count();
    lines.push(String::new());
    lines.push(if bad == 0 {
        "All sources are up to date.".into()
    } else {
        format!(
            "{bad} of {} sources had problems; cached data was kept.",
            report.sources.len()
        )
    });
    lines
}

fn describe_counts(source: &SourceReport) -> String {
    let c = &source.counts;
    let parts: Vec<String> = [
        (c.tickets, "ticket"),
        (c.ticket_comments, "ticket comment"),
        (c.prs, "PR"),
        (c.pr_comments, "PR comment"),
        (c.pipelines, "pipeline run"),
    ]
    .iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}{}", if *n == 1 { "" } else { "s" }))
    .chain((c.removed > 0).then(|| format!("{} stale removed", c.removed)))
    .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(": {}", parts.join(", "))
    }
}

fn describe_source(source: &SourceReport) -> (String, Vec<String>) {
    match &source.outcome {
        SourceOutcome::Synced => (format!("synced{}", describe_counts(source)), vec![]),
        SourceOutcome::Skipped { last_ok_at } => (
            format!(
                "fresh (synced {}); use --force to refresh",
                format_timestamp(*last_ok_at)
            ),
            vec![],
        ),
        SourceOutcome::NotConfigured(reason) => (format!("not configured: {reason}"), vec![]),
        SourceOutcome::Failed(problem) => (
            format!("{}: {} (cached data kept)", problem.kind, problem.message),
            vec![],
        ),
        SourceOutcome::Partial(problems) => (
            format!(
                "partial{}: {} problem{}",
                describe_counts(source),
                problems.len(),
                if problems.len() == 1 { "" } else { "s" }
            ),
            problems
                .iter()
                .map(|p| match &p.subject {
                    Some(s) => format!("- {s}: {}: {}", p.kind, p.message),
                    None => format!("- {}: {}", p.kind, p.message),
                })
                .collect(),
        ),
    }
}

// ------------------------------------------------------ de providers check

/// One provider's answer to "are you usable".
pub struct HealthRow {
    pub name: &'static str,
    /// `Err` when the adapter could not even be built.
    pub health: Result<Health, ProviderError>,
}

/// `de providers check`.
pub fn providers_check() -> eyre::Result<()> {
    let ui = UserInterface::new();
    let config = Config::load()?;
    let providers = Providers::from_config(&config);

    let rows = vec![
        HealthRow {
            name: "jira (acli)",
            health: providers
                .jira
                .as_ref()
                .map(|p| p.health())
                .map_err(Clone::clone),
        },
        HealthRow {
            name: "bitbucket (bkt)",
            health: providers
                .code_host
                .as_ref()
                .map(|p| p.health())
                .map_err(Clone::clone),
        },
    ];
    for line in render_health(&rows) {
        ui.writeln(&line)?;
    }

    if rows
        .iter()
        .any(|r| !r.health.as_ref().is_ok_and(Health::is_ready))
    {
        return Err(eyre!("Some providers are not ready"));
    }
    Ok(())
}

/// Lines for the health check, one block per provider.
pub fn render_health(rows: &[HealthRow]) -> Vec<String> {
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);

    let mut lines = Vec::new();
    for row in rows {
        match &row.health {
            Ok(h) => {
                let version = h.version.as_deref().unwrap_or("-");
                lines.push(format!(
                    "{:<width$}  installed: {}  version: {version}  authenticated: {}  meets minimum: {}",
                    row.name,
                    yes_no(h.installed),
                    yes_no(h.authenticated),
                    yes_no(h.meets_minimum),
                ));
                let status = if h.is_ready() { "ready" } else { "NOT READY" };
                lines.push(format!("{:<width$}  {status}: {}", "", h.detail));
            }
            Err(e) => {
                lines.push(format!(
                    "{:<width$}  unavailable ({}): {e}",
                    row.name,
                    e.kind()
                ));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_core::providers::fake::build::{pr, ticket};
    use de_core::providers::fake::{FakeBitbucket, FakeJira};
    use de_core::providers::{ProviderErrorKind, RemoteTicket};
    use de_core::sync::{Problem, SyncCounts};
    use std::path::PathBuf;

    fn report(source: &str, system: SyncSource, outcome: SourceOutcome) -> SourceReport {
        SourceReport {
            system,
            source: source.into(),
            outcome,
            counts: SyncCounts::default(),
            notes: Vec::new(),
        }
    }

    fn problem(kind: ProviderErrorKind, message: &str, subject: Option<&str>) -> Problem {
        Problem {
            kind,
            message: message.into(),
            subject: subject.map(String::from),
        }
    }

    #[test]
    fn report_renders_every_outcome() {
        let mut synced = report("jira", SyncSource::Jira, SourceOutcome::Synced);
        synced.counts = SyncCounts {
            tickets: 3,
            ticket_comments: 1,
            removed: 2,
            ..Default::default()
        };
        synced
            .notes
            .push("jira.review_jql is not configured".into());
        let offline = report(
            "bitbucket:acme/web",
            SyncSource::Bitbucket,
            SourceOutcome::Failed(problem(
                ProviderErrorKind::Network,
                "network error: offline",
                None,
            )),
        );
        let login = report(
            "bitbucket:acme/api",
            SyncSource::Bitbucket,
            SourceOutcome::Failed(problem(
                ProviderErrorKind::NotAuthenticated,
                "bkt is not authenticated: run bkt auth login",
                None,
            )),
        );
        let partial = report(
            "bitbucket:acme/worker",
            SyncSource::Bitbucket,
            SourceOutcome::Partial(vec![problem(
                ProviderErrorKind::NotFound,
                "not found: acme/worker #4",
                Some("acme/worker #4"),
            )]),
        );
        let skipped = report(
            "bitbucket:acme/docs",
            SyncSource::Bitbucket,
            SourceOutcome::Skipped { last_ok_at: 86_400 },
        );
        let none = report(
            "bitbucket",
            SyncSource::Bitbucket,
            SourceOutcome::NotConfigured("no project has a [hosting] section".into()),
        );

        let lines = render_sync_report(&SyncReport {
            sources: vec![synced, offline, login, partial, skipped, none],
        });
        let text = lines.join("\n");
        assert!(
            text.contains("jira")
                && text.contains("synced: 3 tickets, 1 ticket comment, 2 stale removed"),
            "{text}"
        );
        assert!(
            text.contains("note: jira.review_jql is not configured"),
            "{text}"
        );
        assert!(
            text.contains("offline: network error: offline (cached data kept)"),
            "{text}"
        );
        assert!(
            text.contains("not logged in: bkt is not authenticated"),
            "{text}"
        );
        assert!(text.contains("partial: 1 problem"), "{text}");
        assert!(
            text.contains("- acme/worker #4: not found: not found: acme/worker #4"),
            "{text}"
        );
        assert!(
            text.contains("fresh (synced 1970-01-02 00:00); use --force to refresh"),
            "{text}"
        );
        assert!(
            text.contains("not configured: no project has a [hosting] section"),
            "{text}"
        );
        assert!(
            text.ends_with("3 of 6 sources had problems; cached data was kept."),
            "{text}"
        );

        // Source names line up in a column.
        let col = lines[0].find("synced").unwrap();
        assert!(lines[2].starts_with("bitbucket:acme/web"));
        assert_eq!(lines[2].find("offline").unwrap(), col);
    }

    #[test]
    fn clean_and_empty_reports() {
        assert_eq!(
            render_sync_report(&SyncReport::default()),
            ["Nothing to sync."]
        );
        let lines = render_sync_report(&SyncReport {
            sources: vec![report("jira", SyncSource::Jira, SourceOutcome::Synced)],
        });
        assert_eq!(lines[0], "jira  synced");
        assert_eq!(lines.last().unwrap(), "All sources are up to date.");
    }

    #[test]
    fn health_renders_ready_broken_and_unavailable_providers() {
        let mut broken = Health::ok("1.0");
        broken.authenticated = false;
        broken.meets_minimum = false;
        broken.detail = "run `acli auth login`".into();
        let rows = [
            HealthRow {
                name: "jira (acli)",
                health: Ok(broken),
            },
            HealthRow {
                name: "bitbucket (bkt)",
                health: Ok(Health::ok("0.9.1")),
            },
            HealthRow {
                name: "extra",
                health: Err(ProviderError::not_installed("gh", "adapter not available")),
            },
            HealthRow {
                name: "none",
                health: Ok(Health::not_installed("bkt not found on PATH")),
            },
        ];
        let text = render_health(&rows).join("\n");
        assert!(
            text.contains("installed: yes  version: 1.0  authenticated: no  meets minimum: no"),
            "{text}"
        );
        assert!(text.contains("NOT READY: run `acli auth login`"), "{text}");
        assert!(
            text.contains("installed: yes  version: 0.9.1  authenticated: yes  meets minimum: yes"),
            "{text}"
        );
        assert!(text.contains("ready: ready"), "{text}");
        assert!(
            text.contains(
                "unavailable (not installed): gh is not installed: adapter not available"
            ),
            "{text}"
        );
        assert!(text.contains("installed: no  version: -"), "{text}");
    }

    fn stores() -> (Store, Store) {
        (
            Store::open_in_memory(Kind::State).unwrap(),
            Store::open_in_memory(Kind::Cache).unwrap(),
        )
    }

    #[test]
    fn run_sync_uses_the_providers_and_never_writes() {
        let (state, cache) = stores();
        let config =
            Config::parse("[jira]\nreview_jql = \"q\"\n[bitbucket]\nworkspace = \"acme\"\n")
                .unwrap();
        let jira = FakeJira::new();
        let ticket: RemoteTicket = ticket("PROJ-1", "In Review");
        jira.on_search("q", vec![ticket]);
        let bb = FakeBitbucket::new();
        bb.add_pr(pr("acme/web", 1, "feature/PROJ-1", "develop"));
        let providers = Providers {
            jira: Ok(Box::new(jira.clone())),
            code_host: Ok(Box::new(bb.clone())),
        };
        let repos = [HostedRepo {
            project: "web".into(),
            repo: "acme/web".into(),
            dir: PathBuf::from("/code/web"),
            branches: Default::default(),
            deploy_environment: None,
        }];
        let ctx = SyncContext {
            state: &state,
            cache: &cache,
            config: &config,
            now: 1000,
            force: false,
            min_interval: 60,
        };

        let report = run_sync(&ctx, &providers, &repos, None).unwrap();
        assert!(report.is_ok(), "{report:?}");
        assert_eq!(report.sources.len(), 2);
        assert!(
            render_sync_report(&report)
                .join("\n")
                .contains("All sources are up to date.")
        );
        assert!(jira.log().writes().is_empty() && bb.log().writes().is_empty());

        // Jira only.
        let only = run_sync(&ctx, &providers, &repos, Some(SyncSource::Jira)).unwrap();
        assert_eq!(only.sources.len(), 1);
    }

    #[test]
    fn run_sync_reports_an_unavailable_adapter_per_source() {
        let (state, cache) = stores();
        let config = Config::parse("[jira]\nreview_jql = \"q\"\n").unwrap();
        // What the registry returns until the adapters exist.
        let providers = Providers::from_config(&config);
        let ctx = SyncContext {
            state: &state,
            cache: &cache,
            config: &config,
            now: 1,
            force: false,
            min_interval: 60,
        };
        let report = run_sync(&ctx, &providers, &[], None).unwrap();
        let text = render_sync_report(&report).join("\n");
        assert!(
            text.contains("not installed: acli is not installed"),
            "{text}"
        );
        assert!(
            text.contains("not configured: no project has a [hosting] section"),
            "{text}"
        );
    }
}

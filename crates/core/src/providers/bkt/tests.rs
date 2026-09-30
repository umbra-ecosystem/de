//! Tests for the bkt adapter against a fake runner. Fixtures under `../fixtures/bkt/` are
//! derived from documentation and source, NOT captured from a real install.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::wire::{WState, map_state, parse_rfc3339, parse_version};
use super::*;
use crate::overlay::{CommandOutput, CommandRunner, ExternalCommand};
use crate::providers::{DiffSide, InlineAnchor, PipelineState, ProviderErrorKind};

macro_rules! fx {
    ($n:literal) => {
        include_str!(concat!("../fixtures/bkt/", $n))
    };
}

type Rule = (Vec<String>, Result<RawOutput, RunError>);

#[derive(Default)]
struct Fake {
    rules: Mutex<Vec<Rule>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl Fake {
    fn v(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|s| String::from(*s)).collect()
    }
    fn ok(&self, argv: &[&str], out: &str) {
        let o = RawOutput {
            code: Some(0),
            stdout: out.into(),
            stderr: String::new(),
        };
        self.rules.lock().unwrap().push((Self::v(argv), Ok(o)));
    }
    fn fail(&self, argv: &[&str], code: i32, stderr: &str) {
        let o = RawOutput {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
        };
        self.rules.lock().unwrap().push((Self::v(argv), Ok(o)));
    }
    fn missing(&self, argv: &[&str]) {
        self.rules
            .lock()
            .unwrap()
            .push((Self::v(argv), Err(RunError::NotFound)));
    }
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl BktRunner for Arc<Fake> {
    fn run(&self, argv: &[String]) -> Result<RawOutput, RunError> {
        self.calls.lock().unwrap().push(argv.to_vec());
        let rules = self.rules.lock().unwrap();
        match rules.iter().find(|(a, _)| a == argv) {
            Some((_, r)) => r.clone(),
            None => Ok(RawOutput {
                code: Some(1),
                stdout: String::new(),
                stderr: format!("fake: unexpected argv {argv:?}"),
            }),
        }
    }
}

const AUTH: [&str; 3] = ["auth", "status", "--json"];
const PIPES: &str = "/repositories/acme/web/pipelines/";
const UUID203: &str = "{aaaaaaaa-0000-0000-0000-000000000003}";
const ENC203: &str = "%7Baaaaaaaa-0000-0000-0000-000000000003%7D";

fn setup() -> (Arc<Fake>, BktHost<Arc<Fake>>) {
    let f = Arc::new(Fake::default());
    f.ok(&AUTH, fx!("auth_status_cloud.json"));
    (Arc::clone(&f), BktHost::with_runner(f))
}

fn writer() -> (Arc<Fake>, BktHostWriter<Arc<Fake>>) {
    let f = Arc::new(Fake::default());
    f.ok(&AUTH, fx!("auth_status_cloud.json"));
    (Arc::clone(&f), BktHostWriter::with_runner(f))
}

fn pr_list_argv(state: &'static str) -> [&'static str; 11] {
    [
        "pr",
        "list",
        "--workspace",
        "acme",
        "--repo",
        "web",
        "--state",
        state,
        "--limit",
        "0",
        "--json",
    ]
}

fn pipe_argv(pagelen: &str, page: Option<&str>) -> Vec<String> {
    let mut a = vec![
        "api".to_string(),
        PIPES.into(),
        "--param".into(),
        format!("pagelen={pagelen}"),
        "--param".into(),
        "sort=-created_on".into(),
    ];
    if let Some(p) = page {
        a.push("--param".into());
        a.push(format!("page={p}"));
    }
    a
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn pipes_fixture(f: &Fake) {
    f.ok(&refs(&pipe_argv("100", None)), fx!("pipelines_page1.json"));
    f.ok(
        &refs(&pipe_argv("100", Some("2"))),
        fx!("pipelines_page2.json"),
    );
}

fn kind<T: std::fmt::Debug>(r: ProviderResult<T>) -> ProviderErrorKind {
    r.unwrap_err().kind()
}

// ---- health ----

#[test]
fn health_ready() {
    let (f, h) = setup();
    f.ok(&["--version"], "bkt version 0.32.1\n");
    let health = h.health();
    assert!(health.is_ready(), "{health:?}");
    assert_eq!(health.version.as_deref(), Some("0.32.1"));
    assert!(health.detail.contains("Cloud"));
}

#[test]
fn health_not_installed_old_version_and_auth_states() {
    let (f, h) = setup();
    f.missing(&["--version"]);
    let hl = h.health();
    assert!(!hl.installed && !hl.is_ready());

    let (f, h) = setup();
    f.ok(&["--version"], "bkt version 0.20.0\n");
    let hl = h.health();
    assert!(
        hl.installed && !hl.meets_minimum && !hl.is_ready(),
        "{hl:?}"
    );
    assert!(hl.detail.contains(MIN_BKT_VERSION));

    let f = Arc::new(Fake::default());
    f.ok(&["--version"], "bkt version 0.32.1\n");
    f.ok(&AUTH, fx!("auth_status_empty.json"));
    let hl = BktHost::with_runner(Arc::clone(&f)).health();
    assert!(hl.installed && !hl.authenticated && !hl.is_ready());
    assert!(hl.detail.contains("auth login"));

    let f = Arc::new(Fake::default());
    f.ok(&["--version"], "bkt version 0.32.1\n");
    f.fail(&AUTH, 1, "boom");
    let hl = BktHost::with_runner(f).health();
    assert!(!hl.authenticated && hl.detail.contains("boom"));

    // unparseable version (source build): not blocked
    let (f, h) = setup();
    f.ok(&["--version"], "bkt version dev\n");
    assert!(h.health().is_ready());
}

#[test]
fn health_data_center_is_not_ready() {
    let f = Arc::new(Fake::default());
    f.ok(&["--version"], "bkt version 0.32.1\n");
    f.ok(&AUTH, fx!("auth_status_dc.json"));
    let hl = BktHost::with_runner(f).health();
    assert!(!hl.is_ready());
    assert!(hl.detail.contains("Data Center"), "{}", hl.detail);
}

#[test]
fn constructors_do_not_spawn_and_missing_binary_is_reported() {
    let cfg = Config::default();
    assert!(BktHost::new(&cfg).is_ok());
    assert!(BktHostWriter::new(&cfg).is_ok());
    let r = SystemRunner::new().with_program("definitely-not-a-bkt-binary");
    let h = BktHost::with_runner(r);
    assert!(!h.health().installed);
    assert_eq!(kind(h.pr("acme/web", 1)), ProviderErrorKind::NotInstalled);
}

#[test]
fn adapters_are_send() {
    fn send<T: Send>() {}
    send::<BktHost>();
    send::<BktHostWriter>();
    send::<Box<dyn CodeHost>>();
    send::<Box<dyn CodeHostWriter>>();
}

// ---- reads ----

#[test]
fn list_prs_open_has_destination_and_is_cloud_checked_once() {
    let (f, h) = setup();
    f.ok(&pr_list_argv("OPEN"), fx!("pr_list.json"));
    let prs = h.list_prs("acme/web", &PrFilter::open()).unwrap();
    h.list_prs("acme/web", &PrFilter::open()).unwrap();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0].id, 42);
    assert_eq!(prs[0].destination_branch, "develop");
    assert_eq!(prs[0].source_branch, "feature/PROJ-101-invoice-export");
    assert_eq!(prs[0].state, PrState::Open);
    assert_eq!(prs[0].author, "557058:ann");
    assert_eq!(
        prs[0].url,
        "https://bitbucket.org/acme/web/pull-requests/42"
    );
    assert_eq!(prs[0].reviewers[0].account, "557058:rita");
    assert!(prs[0].updated_at > 1_700_000_000);
    assert_eq!(prs[1].destination_branch, "master");
    let auths = f.calls().iter().filter(|c| c[0] == "auth").count();
    assert_eq!(auths, 1);
}

#[test]
fn list_prs_without_state_asks_every_state_and_filters_text() {
    let (f, h) = setup();
    f.ok(&pr_list_argv("OPEN"), fx!("pr_list.json"));
    f.ok(&pr_list_argv("MERGED"), fx!("pr_list_merged.json"));
    f.ok(&pr_list_argv("DECLINED"), fx!("pr_list_empty.json"));
    f.ok(&pr_list_argv("SUPERSEDED"), fx!("pr_list_empty.json"));
    let all = h.list_prs("acme/web", &PrFilter::default()).unwrap();
    assert_eq!(all.iter().map(|p| p.id).collect::<Vec<_>>(), [42, 43, 40]);
    let hot = PrFilter {
        state: None,
        text: Some("HOTFIX".into()),
    };
    let found = h.list_prs("acme/web", &hot).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, 43);
}

#[test]
fn list_prs_empty() {
    let (f, h) = setup();
    f.ok(&pr_list_argv("OPEN"), fx!("pr_list_empty.json"));
    assert!(
        h.list_prs("acme/web", &PrFilter::open())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pr_carries_destination_and_reviewer_state() {
    let (f, h) = setup();
    f.ok(
        &[
            "pr",
            "view",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web",
            "--json",
        ],
        fx!("pr_view.json"),
    );
    let pr = h.pr("acme/web", 42).unwrap();
    assert_eq!(pr.destination_branch, "master");
    let by = |a: &str| pr.reviewers.iter().find(|r| r.account == a).cloned();
    let rita = by("557058:rita").unwrap();
    assert!(rita.approved && !rita.changes_requested);
    let carl = by("557058:carl").unwrap();
    assert!(!carl.approved && carl.changes_requested);
    let nina = by("557058:nina").unwrap();
    assert!(!nina.approved && !nina.changes_requested);
    // a non-reviewer who approved is included; a plain participant is not
    assert!(by("557058:olga").unwrap().approved);
    assert!(by("557058:ann").is_none());
    assert_eq!(pr.reviewers.len(), 4);
}

#[test]
fn malformed_json_is_parse_error_with_snippet() {
    let (f, h) = setup();
    f.ok(
        &[
            "pr",
            "view",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web",
            "--json",
        ],
        "this is not json at all",
    );
    match h.pr("acme/web", 42).unwrap_err() {
        ProviderError::Parse { tool, what, detail } => {
            assert_eq!(tool, "bkt");
            assert!(what.contains("pull request"));
            assert!(detail.contains("this is not json"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    f.ok(
        &pr_list_argv("OPEN"),
        r#"{"pull_requests":[{"id":5,"state":"WEIRD"}]}"#,
    );
    assert_eq!(
        kind(h.list_prs("acme/web", &PrFilter::open())),
        ProviderErrorKind::Parse
    );
    // wrong type for a known field
    f.ok(&pr_list_argv("MERGED"), r#"{"pull_requests":[{"id":"x"}]}"#);
    let merged = PrFilter {
        state: Some(PrState::Merged),
        text: None,
    };
    assert_eq!(
        kind(h.list_prs("acme/web", &merged)),
        ProviderErrorKind::Parse
    );
}

#[test]
fn pr_comments_ordered_without_deleted_and_with_inline_sides() {
    let (f, h) = setup();
    f.ok(
        &[
            "pr",
            "comments",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web",
            "--json",
        ],
        fx!("pr_comments.json"),
    );
    let c = h.pr_comments("acme/web", 42).unwrap();
    assert_eq!(c.iter().map(|c| c.id).collect::<Vec<_>>(), [901, 902, 904]);
    assert_eq!(c[0].inline, None);
    assert_eq!(
        c[1].inline,
        Some(InlineAnchor {
            path: "src/a.php".into(),
            line: 7,
            side: DiffSide::Old
        })
    );
    assert_eq!(
        c[2].inline,
        Some(InlineAnchor {
            path: "src/b.php".into(),
            line: 12,
            side: DiffSide::New
        })
    );
    assert_eq!(c[0].author, "557058:rita");
    assert_eq!(c[2].author, "");
    assert_eq!(c[0].pr, 42);
}

#[test]
fn pipelines_page_and_filter_by_commit_prefix() {
    let (f, h) = setup();
    pipes_fixture(&f);
    let run = |commit: &str| {
        let filter = PipelineFilter {
            commit: Some(commit.into()),
            ..PipelineFilter::default()
        };
        h.pipelines("acme/web", &filter).unwrap()
    };
    let hit = run("abcdef12");
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].number, Some(203));
    assert_eq!(hit[0].state, PipelineState::Succeeded);
    assert_eq!(hit[0].branch, "uat");
    assert_eq!(hit[0].commit, "abcdef1234567890abcdef1234567890abcdef12");
    assert!(hit[0].created_at > 0 && hit[0].completed_at > Some(hit[0].created_at));
    assert!(hit[0].steps.is_empty());
    assert_eq!(
        hit[0].url,
        "https://bitbucket.org/acme/web/pipelines/results/203"
    );
    // 7 chars matches both abcdef1... runs (found on page 1 and 2)
    assert_eq!(run("abcdef1").len(), 2);
    // fewer than 7 chars never matches a full SHA
    assert!(run("abcdef").is_empty());
    // a commit only on the second page is found (paging combined)
    assert_eq!(run("0123456789abcdef").len(), 1);
    // full SHA
    assert_eq!(
        run("fedcba9876543210fedcba9876543210fedcba98")[0].state,
        PipelineState::Running
    );
}

#[test]
fn pipelines_filter_by_branch_limit_and_unfiltered_paging() {
    let (f, h) = setup();
    pipes_fixture(&f);
    let by_branch = PipelineFilter {
        branch: Some("feature/x".into()),
        ..PipelineFilter::default()
    };
    let r = h.pipelines("acme/web", &by_branch).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].state, PipelineState::Failed);

    let capped = PipelineFilter {
        branch: Some("uat".into()),
        limit: Some(2),
        ..PipelineFilter::default()
    };
    assert_eq!(h.pipelines("acme/web", &capped).unwrap().len(), 2);

    // unfiltered: pagelen follows the limit and continues while `next` is present
    f.ok(&refs(&pipe_argv("3", None)), fx!("pipelines_page1.json"));
    f.ok(
        &refs(&pipe_argv("3", Some("2"))),
        fx!("pipelines_page2.json"),
    );
    let three = PipelineFilter {
        limit: Some(3),
        ..PipelineFilter::default()
    };
    let r = h.pipelines("acme/web", &three).unwrap();
    assert_eq!(
        r.iter().map(|x| x.number).collect::<Vec<_>>(),
        [Some(204), Some(203), Some(202)]
    );
}

#[test]
fn pipeline_scan_is_bounded() {
    let (f, h) = setup();
    // every page claims a next page and never matches
    for p in [None, Some("2"), Some("3"), Some("4"), Some("5"), Some("6")] {
        f.ok(&refs(&pipe_argv("100", p)), fx!("pipelines_page1.json"));
    }
    let filter = PipelineFilter {
        commit: Some("deadbeefdead".into()),
        ..PipelineFilter::default()
    };
    assert!(h.pipelines("acme/web", &filter).unwrap().is_empty());
    let pages = f.calls().iter().filter(|c| c[0] == "api").count();
    assert_eq!(pages, MAX_SCAN_PAGES);
}

fn steps_fixture(f: &Fake) {
    let base = format!("/repositories/acme/web/pipelines/{ENC203}");
    f.ok(&["api", &base], fx!("pipeline_203.json"));
    f.ok(
        &["api", &format!("{base}/steps/"), "--param", "pagelen=100"],
        fx!("pipeline_steps.json"),
    );
    f.ok(
        &[
            "api",
            "/repositories/acme/web/environments",
            "--param",
            "pagelen=100",
        ],
        fx!("environments.json"),
    );
}

#[test]
fn pipeline_steps_and_deployment_environment() {
    let (f, h) = setup();
    steps_fixture(&f);
    let run = h.pipeline("acme/web", UUID203).unwrap();
    assert_eq!(run.number, Some(203));
    assert_eq!(run.steps.len(), 3);
    assert_eq!(run.steps[0].name, "Build");
    assert_eq!(run.steps[0].deployment_environment, None);
    // uuid-only reference resolved through /environments
    assert_eq!(
        run.steps[1].deployment_environment.as_deref(),
        Some("Alpha")
    );
    assert_eq!(run.steps[1].state, PipelineState::Succeeded);
    // name given inline; pending step
    assert_eq!(
        run.steps[2].deployment_environment.as_deref(),
        Some("Production")
    );
    assert_eq!(run.steps[2].state, PipelineState::Pending);
    assert!(run.deployed(None));
    assert!(run.deployed(Some("alpha")));
    assert!(!run.deployed(Some("production")));
    // also addressable by build number
    let base = "/repositories/acme/web/pipelines/203";
    f.ok(&["api", base], fx!("pipeline_203.json"));
    assert!(h.pipeline("acme/web", "203").is_ok());
}

#[test]
fn step_environment_variants() {
    let (f, h) = setup();
    let base = format!("/repositories/acme/web/pipelines/{ENC203}");
    f.ok(&["api", &base], fx!("pipeline_203.json"));
    let steps = r#"{"values":[
        {"name":"a","state":{"name":"COMPLETED","result":{"name":"SUCCESSFUL"}},"environment":"test"},
        {"name":"b","state":{"name":"COMPLETED","result":{"name":"SUCCESSFUL"}},"deployment":{"environment":{"slug":"uat"}}},
        {"name":"c","state":{"name":"COMPLETED","result":{"name":"SUCCESSFUL"}},"environment":null}]}"#;
    f.ok(
        &["api", &format!("{base}/steps/"), "--param", "pagelen=100"],
        steps,
    );
    let run = h.pipeline("acme/web", UUID203).unwrap();
    let envs: Vec<_> = run
        .steps
        .iter()
        .map(|s| s.deployment_environment.clone())
        .collect();
    assert_eq!(envs, [Some("test".into()), Some("uat".into()), None]);
}

#[test]
fn pipeline_state_mapping_table() {
    let st = |n: &str, r: &str, g: &str| {
        let j = json!({"name": n, "result": {"name": r}, "stage": {"name": g}});
        map_state(&serde_json::from_value::<WState>(j).unwrap())
    };
    use PipelineState::*;
    assert_eq!(st("PENDING", "", ""), Pending);
    assert_eq!(st("IN_PROGRESS", "", "RUNNING"), Running);
    assert_eq!(st("IN_PROGRESS", "", ""), Running);
    assert_eq!(st("IN_PROGRESS", "", "PAUSED"), Other("PAUSED".into()));
    assert_eq!(st("COMPLETED", "SUCCESSFUL", ""), Succeeded);
    assert_eq!(st("COMPLETED", "FAILED", ""), Failed);
    assert_eq!(st("COMPLETED", "ERROR", ""), Failed);
    assert_eq!(st("COMPLETED", "STOPPED", ""), Stopped);
    assert_eq!(st("completed", "successful", ""), Succeeded);
    assert_eq!(st("COMPLETED", "", ""), Other("COMPLETED".into()));
    assert_eq!(st("NOT_RUN", "", ""), Other("NOT_RUN".into()));
    assert_eq!(st("", "", ""), Other("UNKNOWN".into()));
}

#[test]
fn helpers_parse_versions_and_times() {
    assert_eq!(parse_version("bkt version 0.32.1"), Some((0, 32, 1)));
    assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
    assert_eq!(parse_version("bkt version dev"), None);
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(
        parse_rfc3339("2001-09-09T01:46:40.123456+00:00"),
        Some(1_000_000_000)
    );
    assert_eq!(
        parse_rfc3339("2001-09-09T03:46:40+02:00"),
        Some(1_000_000_000)
    );
    assert_eq!(parse_rfc3339("garbage"), None);
}

#[test]
fn errors_are_classified() {
    let (f, h) = setup();
    let argv = [
        "pr",
        "view",
        "1",
        "--workspace",
        "acme",
        "--repo",
        "web",
        "--json",
    ];
    let cases = [
        ("HTTP 401 Unauthorized", ProviderErrorKind::NotAuthenticated),
        (
            "Get x: dial tcp: lookup api.bitbucket.org: no such host",
            ProviderErrorKind::Network,
        ),
        ("404 Not Found", ProviderErrorKind::NotFound),
        ("something else", ProviderErrorKind::Command),
        (
            "command supports Bitbucket Cloud contexts only",
            ProviderErrorKind::Unsupported,
        ),
    ];
    for (stderr, want) in cases {
        f.rules
            .lock()
            .unwrap()
            .retain(|(a, _)| a != &Fake::v(&argv));
        f.fail(&argv, 1, stderr);
        assert_eq!(kind(h.pr("acme/web", 1)), want, "{stderr}");
    }
}

#[test]
fn not_authenticated_and_data_center() {
    let f = Arc::new(Fake::default());
    f.ok(&AUTH, fx!("auth_status_empty.json"));
    let h = BktHost::with_runner(Arc::clone(&f));
    assert_eq!(
        kind(h.list_prs("acme/web", &PrFilter::open())),
        ProviderErrorKind::NotAuthenticated
    );

    let f = Arc::new(Fake::default());
    f.ok(&AUTH, fx!("auth_status_dc.json"));
    let h = BktHost::with_runner(Arc::clone(&f));
    assert_eq!(
        kind(h.list_prs("acme/web", &PrFilter::open())),
        ProviderErrorKind::Unsupported
    );
    assert_eq!(kind(h.pr("acme/web", 1)), ProviderErrorKind::Unsupported);
    assert_eq!(
        kind(h.pipelines("acme/web", &PipelineFilter::default())),
        ProviderErrorKind::Unsupported
    );
    let w = BktHostWriter::with_runner(Arc::clone(&f));
    assert_eq!(
        kind(w.approve("acme/web", 1)),
        ProviderErrorKind::Unsupported
    );
    let t = TriggerSpec {
        branch: "uat".into(),
        commit: None,
        custom_pipeline: None,
    };
    assert_eq!(
        kind(w.trigger_pipeline("acme/web", &t)),
        ProviderErrorKind::Unsupported
    );
    assert!(
        f.calls().iter().all(|c| c[..] == Fake::v(&AUTH)[..]),
        "only the local auth check runs"
    );
}

#[test]
fn bad_repo_and_ids_are_rejected_without_running_anything() {
    let (f, h) = setup();
    assert_eq!(kind(h.pr("no-slash", 1)), ProviderErrorKind::Other);
    assert_eq!(kind(h.pr("a/b c", 1)), ProviderErrorKind::Other);
    assert_eq!(
        kind(h.pipeline("acme/web", "x; rm -rf")),
        ProviderErrorKind::Other
    );
    assert!(f.calls().iter().all(|c| c[0] != "pr" && c[0] != "api"));
}

#[test]
fn reads_never_issue_write_commands() {
    let (f, h) = setup();
    f.ok(&["--version"], "bkt version 0.32.1");
    f.ok(&pr_list_argv("OPEN"), fx!("pr_list.json"));
    f.ok(
        &[
            "pr",
            "view",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web",
            "--json",
        ],
        fx!("pr_view.json"),
    );
    f.ok(
        &[
            "pr",
            "comments",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web",
            "--json",
        ],
        fx!("pr_comments.json"),
    );
    f.ok(&refs(&pipe_argv("30", None)), fx!("pipelines_page2.json"));
    steps_fixture(&f);
    h.health();
    h.list_prs("acme/web", &PrFilter::open()).unwrap();
    h.pr("acme/web", 42).unwrap();
    h.pr_comments("acme/web", 42).unwrap();
    h.pipelines("acme/web", &PipelineFilter::default()).unwrap();
    h.pipeline("acme/web", UUID203).unwrap();
    let calls = f.calls();
    assert!(calls.len() > 8);
    for c in calls {
        let head = (c[0].as_str(), c.get(1).map(String::as_str));
        let allowed = matches!(
            head,
            ("--version", _)
                | ("auth", Some("status"))
                | ("pr", Some("list" | "view" | "comments"))
                | ("api", _)
        );
        assert!(allowed, "unexpected read command {c:?}");
        assert!(
            !c.iter()
                .any(|a| a == "--method" || a == "-X" || a == "--input" || a == "-d"),
            "{c:?}"
        );
    }
}

// ---- writes ----

fn comment_argv(id: u64) -> [String; 4] {
    [
        "api".into(),
        format!("/repositories/acme/web/pullrequests/{id}/comments"),
        "--method".into(),
        "POST".into(),
    ]
}

fn post_call(f: &Fake, idx: usize) -> (Vec<String>, Value) {
    let c = f.calls()[idx].clone();
    assert_eq!(c[c.len() - 2], "--input");
    let body: Value = serde_json::from_str(&c[c.len() - 1]).unwrap();
    (c[..c.len() - 2].to_vec(), body)
}

fn reg_post(f: &Fake, id: u64, body: &Value, reply: &str) {
    let mut argv: Vec<String> = comment_argv(id).into();
    argv.push("--input".into());
    argv.push(body.to_string());
    f.rules.lock().unwrap().push((
        argv,
        Ok(RawOutput {
            code: Some(0),
            stdout: reply.into(),
            stderr: String::new(),
        }),
    ));
}

#[test]
fn inline_comment_posts_verbatim_with_correct_shape() {
    let (f, w) = writer();
    let text = "line \"one\"\nline two \u{2713} `x` $HOME 'q' \\n\t";
    let body = json!({"content": {"raw": text}, "inline": {"path": "src/a.php", "to": 12}});
    reg_post(&f, 42, &body, fx!("comment_created_inline.json"));
    let anchor = InlineAnchor {
        path: "src/a.php".into(),
        line: 12,
        side: DiffSide::New,
    };
    let c = w
        .add_pr_comment(
            "acme/web",
            42,
            &NewPrComment {
                body: text.into(),
                inline: Some(anchor.clone()),
            },
        )
        .unwrap();
    assert_eq!(c.id, 950);
    assert_eq!(c.inline, Some(anchor));
    let (head, sent) = post_call(&f, 1);
    assert_eq!(head, comment_argv(42));
    assert_eq!(sent["content"]["raw"], text);
    assert_eq!(sent, body);
}

#[test]
fn old_side_uses_from_and_general_has_no_inline() {
    let (f, w) = writer();
    let body = json!({"content": {"raw": "x"}, "inline": {"path": "a/b.rs", "from": 3}});
    reg_post(&f, 7, &body, fx!("comment_created_inline.json"));
    let anchor = InlineAnchor {
        path: "a/b.rs".into(),
        line: 3,
        side: DiffSide::Old,
    };
    w.add_pr_comment(
        "acme/web",
        7,
        &NewPrComment {
            body: "x".into(),
            inline: Some(anchor),
        },
    )
    .unwrap();
    assert_eq!(post_call(&f, 1).1, body);

    let (f, w) = writer();
    let body = json!({"content": {"raw": "hello"}});
    reg_post(&f, 7, &body, fx!("comment_created_general.json"));
    let c = w
        .add_pr_comment(
            "acme/web",
            7,
            &NewPrComment {
                body: "hello".into(),
                inline: None,
            },
        )
        .unwrap();
    assert_eq!(c.id, 951);
    assert!(c.inline.is_none());
    assert!(post_call(&f, 1).1.get("inline").is_none());
}

#[test]
fn comment_validation_and_unreadable_reply() {
    let (f, w) = writer();
    let bad = |body: &str, inline: Option<InlineAnchor>| NewPrComment {
        body: body.into(),
        inline,
    };
    assert!(w.add_pr_comment("acme/web", 1, &bad("  ", None)).is_err());
    let zero = InlineAnchor {
        path: "a".into(),
        line: 0,
        side: DiffSide::New,
    };
    assert!(
        w.add_pr_comment("acme/web", 1, &bad("x", Some(zero)))
            .is_err()
    );
    let nopath = InlineAnchor {
        path: " ".into(),
        line: 1,
        side: DiffSide::New,
    };
    assert!(
        w.add_pr_comment("acme/web", 1, &bad("x", Some(nopath)))
            .is_err()
    );
    assert!(f.calls().is_empty(), "nothing is sent for invalid input");

    reg_post(&f, 1, &json!({"content": {"raw": "x"}}), "not json");
    let e = w
        .add_pr_comment("acme/web", 1, &bad("x", None))
        .unwrap_err();
    assert!(e.to_string().contains("probably posted"), "{e}");
}

#[test]
fn approve_argv() {
    let (f, w) = writer();
    f.ok(
        &[
            "pr",
            "approve",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web",
        ],
        "approved",
    );
    w.approve("acme/web", 42).unwrap();
    assert_eq!(
        f.calls()[1],
        Fake::v(&[
            "pr",
            "approve",
            "42",
            "--workspace",
            "acme",
            "--repo",
            "web"
        ])
    );
}

#[test]
fn request_changes_comments_first_then_requests() {
    let (f, w) = writer();
    let body = json!({"content": {"raw": "please fix"}});
    reg_post(&f, 5, &body, fx!("comment_created_general.json"));
    let rc = "/repositories/acme/web/pullrequests/5/request-changes";
    f.ok(&["api", rc, "--method", "POST"], "{}");
    w.request_changes("acme/web", 5, "please fix").unwrap();
    let calls = f.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2], Fake::v(&["api", rc, "--method", "POST"]));

    // no body: only the state change
    let (f, w) = writer();
    f.ok(&["api", rc, "--method", "POST"], "{}");
    w.request_changes("acme/web", 5, " ").unwrap();
    assert_eq!(f.calls().len(), 2);

    // failure after the comment says the comment exists
    let (f, w) = writer();
    reg_post(&f, 5, &body, fx!("comment_created_general.json"));
    f.fail(&["api", rc, "--method", "POST"], 1, "boom");
    let e = w.request_changes("acme/web", 5, "please fix").unwrap_err();
    assert!(e.to_string().contains("comment 951 was posted"), "{e}");
}

fn pipe_post(f: &Fake, target: &Value, reply: &str) {
    let mut argv = Fake::v(&["api", PIPES, "--method", "POST", "--input"]);
    argv.push(json!({ "target": target }).to_string());
    f.rules.lock().unwrap().push((
        argv,
        Ok(RawOutput {
            code: Some(0),
            stdout: reply.into(),
            stderr: String::new(),
        }),
    ));
}

#[test]
fn trigger_pipeline_bodies() {
    let (f, w) = writer();
    let target = json!({"type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "uat"});
    pipe_post(&f, &target, fx!("pipeline_203.json"));
    let run = w
        .trigger_pipeline(
            "acme/web",
            &TriggerSpec {
                branch: "uat".into(),
                commit: None,
                custom_pipeline: None,
            },
        )
        .unwrap();
    assert_eq!(run.number, Some(203));

    let sha = "abcdef1234567890abcdef1234567890abcdef12";
    let target = json!({"type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "uat",
        "commit": {"type": "commit", "hash": sha}, "selector": {"type": "custom", "pattern": "deploy-alpha"}});
    pipe_post(&f, &target, fx!("pipeline_203.json"));
    let spec = TriggerSpec {
        branch: "uat".into(),
        commit: Some(sha.into()),
        custom_pipeline: Some("deploy-alpha".into()),
    };
    w.trigger_pipeline("acme/web", &spec).unwrap();

    let before = f.calls().len();
    let short = TriggerSpec {
        branch: "uat".into(),
        commit: Some("abcdef1".into()),
        custom_pipeline: None,
    };
    assert_eq!(
        kind(w.trigger_pipeline("acme/web", &short)),
        ProviderErrorKind::Other
    );
    let nobranch = TriggerSpec {
        branch: " ".into(),
        commit: None,
        custom_pipeline: None,
    };
    assert!(w.trigger_pipeline("acme/web", &nobranch).is_err());
    assert_eq!(f.calls().len(), before);
}

#[test]
fn rerun_retriggers_the_same_target() {
    let (f, w) = writer();
    f.ok(
        &["api", &format!("/repositories/acme/web/pipelines/{ENC203}")],
        fx!("pipeline_203.json"),
    );
    let target = json!({"type": "pipeline_ref_target", "ref_type": "branch", "ref_name": "uat",
        "commit": {"type": "commit", "hash": "abcdef1234567890abcdef1234567890abcdef12"},
        "selector": {"type": "branch", "pattern": "uat"}});
    pipe_post(&f, &target, fx!("pipeline_203.json"));
    let run = w.rerun_pipeline("acme/web", UUID203).unwrap();
    assert_eq!(run.id, UUID203);

    // pull-request pipelines cannot be re-triggered this way
    let (f, w) = writer();
    let pr = r#"{"uuid":"{x}","state":{"name":"PENDING"},"target":{"type":"pipeline_pullrequest_target"}}"#;
    f.ok(
        &["api", &format!("/repositories/acme/web/pipelines/{ENC203}")],
        pr,
    );
    assert_eq!(
        kind(w.rerun_pipeline("acme/web", UUID203)),
        ProviderErrorKind::Unsupported
    );
}

// ---- probe ----

struct ProbeFake {
    calls: Mutex<Vec<String>>,
}

impl CommandRunner for ProbeFake {
    fn run(&self, c: &ExternalCommand) -> eyre::Result<CommandOutput> {
        self.calls.lock().unwrap().push(c.label.clone());
        let line = c.args.join(" ");
        let out = if line.starts_with("pr list") {
            fx!("pr_list.json").to_string()
        } else if line.starts_with("api /repositories/acme/web/pipelines/ ") {
            fx!("pipelines_page1.json").to_string()
        } else if line.starts_with("--version") {
            return Err(eyre::eyre!("spawn failed"));
        } else {
            "{}".to_string()
        };
        Ok(CommandOutput {
            success: true,
            code: Some(0),
            stdout: out,
            stderr: String::new(),
        })
    }
}

#[test]
fn probe_runs_only_read_only_commands_and_derives_ids() {
    let r = ProbeFake {
        calls: Mutex::new(Vec::new()),
    };
    let none = probe(&r, None);
    assert_eq!(none.len(), 2);
    assert_eq!(none[0].exit_code, None);
    assert!(none[0].stderr.contains("spawn failed"));

    let out = probe(&r, Some("acme/web"));
    let cmds: Vec<&str> = out.iter().map(|p| p.command.as_str()).collect();
    assert!(
        cmds.contains(&"bkt pr view 42 --workspace acme --repo web --json"),
        "{cmds:?}"
    );
    assert!(cmds.contains(&"bkt pr comments 42 --workspace acme --repo web --json"));
    assert!(cmds.iter().any(|c| c.contains("/steps/")));
    assert!(cmds.iter().any(|c| c.contains("/environments")));
    for c in &cmds {
        assert!(
            !c.contains("approve")
                && !c.contains("--method")
                && !c.contains("--input")
                && !c.contains("pipeline run"),
            "{c}"
        );
    }
    assert!(!cmds.iter().any(|c| c.starts_with("bkt pr comment ")));
}

// ---- verified against a real bkt 0.32.1 (not logged in) ----

#[test]
fn version_line_of_the_real_bkt_is_parsed() {
    assert_eq!(parse_version("bkt version 0.32.1\n"), Some((0, 32, 1)));
}

#[test]
fn null_auth_status_is_not_authenticated_and_not_a_parse_error() {
    // Real output when logged out: exit 0, `{"hosts": null, "contexts": null}`.
    assert_eq!(
        fx!("auth_status_null.json").trim(),
        r#"{"hosts": null, "contexts": null}"#
    );
    let f = Arc::new(Fake::default());
    f.ok(&["--version"], "bkt version 0.32.1\n");
    f.ok(&AUTH, fx!("auth_status_null.json"));
    let hl = BktHost::with_runner(Arc::clone(&f)).health();
    assert!(hl.installed && hl.meets_minimum, "{hl:?}");
    assert!(!hl.authenticated && !hl.is_ready(), "{hl:?}");
    assert!(
        hl.detail.contains("auth login") && !hl.detail.contains("could not parse"),
        "{}",
        hl.detail
    );
    // A data call takes the same path and is NotAuthenticated, never Parse.
    let h = BktHost::with_runner(f);
    assert_eq!(
        kind(h.list_prs("acme/web", &PrFilter::open())),
        ProviderErrorKind::NotAuthenticated
    );
}

#[test]
fn no_active_context_is_not_authenticated_with_an_actionable_hint() {
    let (f, h) = setup();
    let argv = [
        "pr",
        "view",
        "1",
        "--workspace",
        "acme",
        "--repo",
        "web",
        "--json",
    ];
    f.fail(
        &argv,
        1,
        "Error: no active context; run `bkt context use <name>`\n",
    );
    let err = h.pr("acme/web", 1).unwrap_err();
    assert_eq!(err.kind(), ProviderErrorKind::NotAuthenticated);
    assert!(err.is_environmental());
    let text = err.to_string();
    assert!(
        text.contains("bkt auth login") && text.contains("bkt context use"),
        "{text}"
    );
}

// ---- live (opt-in): cargo test -p de-core -- --ignored live_ ----

#[test]
#[ignore = "runs the real installed bkt (read-only)"]
fn live_health_reports_installed_but_not_authenticated_without_a_login() {
    let h = BktHost::with_runner(SystemRunner::new());
    let hl = h.health();
    if !hl.installed {
        eprintln!("bkt is not installed; skipping");
        return;
    }
    if hl.authenticated {
        eprintln!("a bkt login exists; skipping the logged-out assertions");
        return;
    }
    assert!(hl.meets_minimum, "version {:?}", hl.version);
    assert!(!hl.is_ready());
    assert!(!hl.detail.contains("could not parse"), "{}", hl.detail);
}

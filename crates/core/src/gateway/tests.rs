//! Gateway tests: real SQLite for the audit log, the provider fakes for the writers.

use std::path::{Path, PathBuf};

use super::*;
use crate::{
    domain::AuditOutcome,
    providers::{
        NewPrComment, ProviderError, TriggerSpec,
        fake::{
            FakeBitbucket, FakeJira,
            build::{key, pr, ticket},
        },
    },
    store::{Kind, Store, audit},
};

struct World {
    state: Store,
    jira: FakeJira,
    bb: FakeBitbucket,
}

impl World {
    fn new() -> Self {
        let jira = FakeJira::new();
        jira.add_ticket(ticket("PROJ-1", "In Review"));
        let bb = FakeBitbucket::new();
        bb.add_pr(pr("acme/web", 7, "feature/PROJ-1", "develop"));
        Self {
            state: Store::open_in_memory(Kind::State).unwrap(),
            jira,
            bb,
        }
    }

    fn gateway(&self) -> Gateway<'_> {
        Gateway::with_writers(
            &self.state,
            Ok(Box::new(self.jira.clone())),
            Ok(Box::new(self.bb.clone())),
        )
    }

    fn audit(&self) -> Vec<(String, AuditOutcome)> {
        let mut rows: Vec<_> = audit::list(&self.state, None, 100)
            .unwrap()
            .into_iter()
            .map(|e| (e.action, e.outcome))
            .collect();
        rows.reverse();
        rows
    }
}

fn comment(body: &str) -> Action {
    Action::PostJiraComment {
        ticket: key("PROJ-1"),
        body: body.into(),
    }
}

#[test]
fn a_confirmed_comment_is_sent_verbatim_once_and_audited_before_and_after() {
    let w = World::new();
    let gw = w.gateway();
    let body = "  Deployed.\n\nweb: https://x/1  \n";

    let draft = gw.draft_because(comment(body), serde_json::json!({"why": "all deployed"}));
    assert_eq!(
        draft.preview().payload,
        body,
        "the preview is the literal payload"
    );
    assert_eq!(draft.preview().risk, Risk::Medium);
    assert!(w.jira.log().writes().is_empty(), "drafting sends nothing");
    assert!(w.audit().is_empty(), "drafting audits nothing");

    let executed = gw.execute(draft.confirm(), 50).unwrap();
    assert!(executed.audit_warnings.is_empty());
    assert_eq!(w.jira.log().count("add_comment"), 1);
    assert_eq!(
        w.jira.log().args_of("add_comment"),
        [format!("PROJ-1: {body}")]
    );

    assert_eq!(
        w.audit(),
        [
            ("jira.comment.attempted".to_string(), AuditOutcome::Skipped),
            ("jira.comment".to_string(), AuditOutcome::Success),
        ]
    );
    let entries = audit::list(&w.state, None, 10).unwrap();
    let attempted = &entries[1];
    assert_eq!(
        attempted.details["payload"]["post_jira_comment"]["body"],
        body
    );
    assert_eq!(attempted.details["gateway"]["facts"]["why"], "all deployed");
    assert_eq!(attempted.ticket, Some(key("PROJ-1")));
}

#[test]
fn a_payload_changed_after_the_preview_is_rejected_and_nothing_is_written() {
    let w = World::new();
    let gw = w.gateway();
    let mut confirmed = gw.draft(comment("what the human saw")).confirm();

    // Someone swaps the payload between preview and execution.
    confirmed.action = comment("something else");
    let err = gw.execute(confirmed, 1).unwrap_err();
    assert!(matches!(err, GatewayError::PayloadChanged { .. }), "{err}");
    assert!(w.jira.log().writes().is_empty());
    assert!(w.audit().is_empty());

    // Same for a pushed-to repo list.
    let mut confirmed = gw
        .draft(Action::ApprovePr {
            repo: "acme/web".into(),
            pr: 7,
        })
        .confirm();
    confirmed.action = Action::ApprovePr {
        repo: "acme/web".into(),
        pr: 8,
    };
    assert!(gw.execute(confirmed, 1).is_err());
    assert!(w.bb.log().writes().is_empty());
}

#[test]
fn each_confirmation_is_for_the_action_it_previewed() {
    // `execute` takes `Confirmed` by value and only `Draft::confirm` makes one: these are
    // compile-time guarantees. What can be checked at run time: two drafts of the same
    // payload are separate confirmations, each sending exactly once.
    let w = World::new();
    let gw = w.gateway();
    let a = gw.draft(comment("same"));
    let b = gw.draft(comment("same"));
    assert_ne!(a.id(), b.id());
    assert_eq!(a.payload_hash(), b.payload_hash());
    gw.execute(a.confirm(), 1).unwrap();
    assert_eq!(w.jira.log().count("add_comment"), 1);
    gw.execute(b.confirm(), 2).unwrap();
    assert_eq!(w.jira.log().count("add_comment"), 2);
}

#[test]
fn if_the_attempt_cannot_be_audited_nothing_is_done() {
    let w = World::new();
    w.state
        .conn()
        .execute_batch(
            "CREATE TRIGGER no_audit BEFORE INSERT ON audit_log
             BEGIN SELECT RAISE(ABORT, 'audit disk full'); END;",
        )
        .unwrap();
    let gw = w.gateway();
    for action in [
        comment("x"),
        Action::TransitionJira {
            ticket: key("PROJ-1"),
            to_status: "Alpha Testing".into(),
        },
        Action::ApprovePr {
            repo: "acme/web".into(),
            pr: 7,
        },
    ] {
        let err = gw.execute(gw.draft(action).confirm(), 1).unwrap_err();
        assert!(matches!(err, GatewayError::Audit(_)), "{err}");
        assert!(err.to_string().contains("audit disk full"), "{err}");
    }
    assert!(w.jira.log().writes().is_empty());
    assert!(w.bb.log().writes().is_empty());
}

#[test]
fn an_unauditable_outcome_is_reported_not_hidden() {
    let w = World::new();
    w.state
        .conn()
        .execute_batch(
            "CREATE TRIGGER no_outcome BEFORE INSERT ON audit_log
             WHEN NEW.action NOT LIKE '%.attempted'
             BEGIN SELECT RAISE(ABORT, 'nope'); END;",
        )
        .unwrap();
    let gw = w.gateway();
    let executed = gw.execute(gw.draft(comment("hi")).confirm(), 1).unwrap();
    assert_eq!(w.jira.log().count("add_comment"), 1);
    assert_eq!(executed.audit_warnings.len(), 1);
    assert_eq!(
        w.audit(),
        [("jira.comment.attempted".to_string(), AuditOutcome::Skipped)]
    );
}

#[test]
fn a_provider_failure_is_returned_and_audited_as_a_failure() {
    let w = World::new();
    w.jira.set_offline();
    let gw = w.gateway();
    let err = gw
        .execute(gw.draft(comment("hi")).confirm(), 1)
        .unwrap_err();
    assert!(matches!(err, GatewayError::Provider(_)), "{err}");
    assert_eq!(
        w.audit(),
        [
            ("jira.comment.attempted".to_string(), AuditOutcome::Skipped),
            ("jira.comment".to_string(), AuditOutcome::Failure),
        ]
    );
    let last = &audit::list(&w.state, None, 1).unwrap()[0];
    assert!(last.details["error"].as_str().unwrap().contains("offline"));
}

#[test]
fn an_unavailable_adapter_fails_clearly_before_anything_is_audited() {
    let w = World::new();
    let unavailable =
        || ProviderError::not_installed("acli", "adapter not available in this build");
    let gw = Gateway::with_writers(&w.state, Err(unavailable()), Ok(Box::new(w.bb.clone())));
    let action = comment("hi");
    let err = gw.ensure_available(&action).unwrap_err();
    assert!(err.to_string().contains("not available"), "{err}");
    let err = gw.execute(gw.draft(action).confirm(), 1).unwrap_err();
    assert!(matches!(err, GatewayError::WriterUnavailable(_)));
    assert!(w.audit().is_empty());
    // The other writer still works.
    gw.execute(
        gw.draft(Action::ApprovePr {
            repo: "acme/web".into(),
            pr: 7,
        })
        .confirm(),
        1,
    )
    .unwrap();
    assert_eq!(w.bb.log().count("approve"), 1);
}

#[test]
fn every_action_kind_reaches_its_writer_with_the_previewed_payload() {
    let w = World::new();
    let gw = w.gateway();
    let inline = NewPrComment {
        body: "nit".into(),
        inline: None,
    };
    let actions = vec![
        Action::TransitionJira {
            ticket: key("PROJ-1"),
            to_status: "Alpha Testing".into(),
        },
        Action::PostPrComment {
            repo: "acme/web".into(),
            pr: 7,
            comment: inline,
        },
        Action::ApprovePr {
            repo: "acme/web".into(),
            pr: 7,
        },
        Action::RequestChanges {
            repo: "acme/web".into(),
            pr: 7,
            body: "fix".into(),
        },
        Action::TriggerPipeline {
            repo: "acme/web".into(),
            spec: TriggerSpec {
                branch: "uat".into(),
                commit: None,
                custom_pipeline: None,
            },
        },
    ];
    for a in actions {
        let draft = gw.draft(a);
        assert!(!draft.preview().payload.is_empty());
        gw.execute(draft.confirm(), 1).unwrap();
    }
    assert_eq!(
        w.jira.log().args_of("transition"),
        ["PROJ-1 -> Alpha Testing"]
    );
    let methods: Vec<_> = w.bb.log().writes().iter().map(|c| c.method).collect();
    assert_eq!(
        methods,
        [
            "add_pr_comment",
            "approve",
            "request_changes",
            "trigger_pipeline"
        ]
    );
    // Two audit rows per action.
    assert_eq!(w.audit().len(), 10);
}

#[test]
fn a_push_is_high_risk_and_previews_the_exact_git_commands() {
    let push = Action::PushUat(PushUat {
        ticket: key("PROJ-1"),
        repos: vec![PushUatRepo {
            repo: "web".into(),
            repo_dir: PathBuf::from("/x/web"),
            remote: "origin".into(),
            uat_branch: "uat".into(),
            ticket_branch: "feature/PROJ-1-x".into(),
            ticket_tip: "a".repeat(40),
            uat_before: "b".repeat(40),
            merge_commit: "c".repeat(40),
            commits: vec![CommitLine {
                sha: "a".repeat(40),
                summary: "work".into(),
            }],
            files: vec!["app.php".into()],
            overlay_packages: vec![],
        }],
    });
    let preview = push.preview();
    assert_eq!(preview.risk, Risk::High);
    assert!(preview.payload.contains(&format!(
        "git push origin {}:refs/heads/uat",
        "c".repeat(40)
    )));
    let text = preview.to_lines().join("\n");
    assert!(text.contains("feature/PROJ-1-x") && text.contains("app.php") && text.contains("work"));
    assert!(text.contains(&"b".repeat(40)) && text.contains("risk: high"));
}

#[cfg(unix)]
#[test]
fn a_payload_that_cannot_be_serialised_is_refused_not_hashed_as_nothing() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let w = World::new();
    let gw = w.gateway();
    // A checkout path that is not UTF-8 cannot be written to the audit log or hashed.
    let dir = PathBuf::from(OsString::from_vec(b"/x/\xff\xfe".to_vec()));
    let push = Action::PushUat(PushUat {
        ticket: key("PROJ-1"),
        repos: vec![PushUatRepo {
            repo: "web".into(),
            repo_dir: dir,
            remote: "origin".into(),
            uat_branch: "uat".into(),
            ticket_branch: "feature/PROJ-1-x".into(),
            ticket_tip: "a".repeat(40),
            uat_before: "b".repeat(40),
            merge_commit: "c".repeat(40),
            commits: vec![],
            files: vec![],
            overlay_packages: vec![],
        }],
    });
    let err = gw.execute(gw.draft(push).confirm(), 1).unwrap_err();
    assert!(matches!(err, GatewayError::Audit(_)), "{err}");
    assert!(
        w.audit().is_empty(),
        "no attempt without a payload to record"
    );
}

// ------------------------------------------------------------ source scan

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_the_gateway_and_the_providers_build_writers() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(files.len() > 50, "the scan found the sources");

    let mut offenders = Vec::new();
    for file in files {
        let path = file.canonicalize().unwrap();
        let text = path.to_string_lossy();
        if text.contains("/src/gateway/")
            || text.contains("/src/providers/")
            || text.contains("/target/")
        {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "build_ticket_writer",
            "build_code_host_writer",
            // The concrete writers are `pub(crate)`; also keep them out of the rest of core.
            "AcliJiraWriter",
            "BktHostWriter",
        ] {
            if source.contains(needle) {
                offenders.push(format!("{text} uses {needle}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "writers built outside the gateway: {offenders:#?}"
    );
}

/// Nothing outside the gateway runs `git push` (a source scan: it cannot catch a command
/// assembled from pieces, a macro or `include!`, but it catches the plain spellings, which is
/// how an accident would happen).
#[test]
fn only_the_gateway_runs_git_push() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let path = file.canonicalize().unwrap().to_string_lossy().into_owned();
        let is_test_code = path.ends_with("tests.rs")
            || path.ends_with("_tests.rs")
            || path.ends_with("testsupport.rs")
            || path.ends_with("testutil.rs");
        if path.contains("/src/gateway/") || path.contains("/target/") || is_test_code {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for (n, line) in source.lines().enumerate() {
            // `git stash push` is local; a `#[cfg(test)]` module inside a file is not scanned
            // separately, so test-only pushes in production files would need an exemption here.
            //
            // The widgets crate is exempt: it has no git (or any process) to run — it draws the
            // preview text of an action the gateway would confirm, which is plain display.
            if (line.contains("\"push\"") || line.contains("git push"))
                && !line.contains("\"stash\"")
                && !path.contains("/crates/widgets/")
            {
                offenders.push(format!("{path}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}

#[test]
fn only_finalize_can_mark_a_ticket_integrated() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let path = file.canonicalize().unwrap().to_string_lossy().into_owned();
        let allowed = [
            "/src/integration/",
            "/src/store/tickets.rs",
            "/src/activation/",
        ]
        .iter()
        .any(|a| path.contains(a))
            || path.ends_with("store/domain_tests.rs")
            || path.ends_with("gateway/tests.rs");
        if allowed {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for needle in ["mark_integrated", "deactivate_for_integration"] {
            if source.contains(needle) {
                offenders.push(format!("{path} uses {needle}"));
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}

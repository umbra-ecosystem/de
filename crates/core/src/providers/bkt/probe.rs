//! `probe`: runs only read-only, harmless `bkt` commands and returns the raw output so the
//! parsers can be verified or repaired against a real install.

use serde_json::Value;

use crate::overlay::{CommandRunner, ExternalCommand};

/// One probed command and what it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// The full command line, for pasting.
    pub command: String,
    /// `None` when the process could not be started (or was killed by a signal).
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

fn run(runner: &dyn CommandRunner, args: &[&str], out: &mut Vec<ProbeResult>) -> Option<String> {
    let label = format!("bkt {}", args.join(" "));
    let cmd = ExternalCommand::new(".", "bkt", args, label.as_str());
    match runner.run(&cmd) {
        Ok(o) => {
            let stdout = o.stdout.clone();
            out.push(ProbeResult {
                command: label,
                exit_code: o.code,
                stdout: o.stdout,
                stderr: o.stderr,
            });
            o.success.then_some(stdout)
        }
        Err(e) => {
            out.push(ProbeResult {
                command: label,
                exit_code: None,
                stdout: String::new(),
                stderr: format!("{e:#}"),
            });
            None
        }
    }
}

fn first_pr_id(stdout: &str) -> Option<u64> {
    let v: Value = serde_json::from_str(stdout).ok()?;
    v.get("pull_requests")?.get(0)?.get("id")?.as_u64()
}

fn first_pipeline_uuid(stdout: &str) -> Option<String> {
    let v: Value = serde_json::from_str(stdout).ok()?;
    Some(v.get("values")?.get(0)?.get("uuid")?.as_str()?.to_string())
}

/// Runs the read-only probe. With `repo` (`workspace/slug`) it also lists a few PRs and
/// pipelines and views the first of each; without it, only version and auth status.
pub fn probe(runner: &dyn CommandRunner, repo: Option<&str>) -> Vec<ProbeResult> {
    let mut out = Vec::new();
    run(runner, &["--version"], &mut out);
    run(runner, &["auth", "status", "--json"], &mut out);
    let Some((ws, slug)) = repo.and_then(|r| r.split_once('/')) else {
        return out;
    };

    let listed = run(
        runner,
        &["pr", "list", "--workspace", ws, "--repo", slug, "--state", "OPEN", "--limit", "3", "--json"],
        &mut out,
    );
    if let Some(id) = listed.as_deref().and_then(first_pr_id) {
        let id = id.to_string();
        run(runner, &["pr", "view", &id, "--workspace", ws, "--repo", slug, "--json"], &mut out);
        run(runner, &["pr", "comments", &id, "--workspace", ws, "--repo", slug, "--json"], &mut out);
    }

    // The CLI's own pipeline list (to show what it drops) and the raw REST list the adapter uses.
    run(
        runner,
        &["pipeline", "list", "--workspace", ws, "--repo", slug, "--limit", "3", "--json"],
        &mut out,
    );
    let base = format!("/repositories/{ws}/{slug}");
    let pipelines = run(
        runner,
        &["api", &format!("{base}/pipelines/"), "--param", "pagelen=3", "--param", "sort=-created_on"],
        &mut out,
    );
    if let Some(uuid) = pipelines.as_deref().and_then(first_pipeline_uuid) {
        let enc = uuid.replace('{', "%7B").replace('}', "%7D");
        run(runner, &["api", &format!("{base}/pipelines/{enc}")], &mut out);
        run(
            runner,
            &["api", &format!("{base}/pipelines/{enc}/steps/"), "--param", "pagelen=100"],
            &mut out,
        );
    }
    run(
        runner,
        &["api", &format!("{base}/environments"), "--param", "pagelen=100"],
        &mut out,
    );
    out
}

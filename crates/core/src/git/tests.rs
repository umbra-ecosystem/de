//! Behavioural tests against real temporary git repositories.

use std::path::PathBuf;

use tempfile::TempDir;

use super::testutil::{TestRepo, fresh_path};
use super::*;

fn names(matches: &[BranchMatch]) -> Vec<&str> {
    matches.iter().map(|m| m.branch.name.as_str()).collect()
}

// ---------------------------------------------------------------- status

#[test]
fn status_clean_repo_without_upstream() {
    let repo = TestRepo::init();
    let head = repo.commit("a.txt", "one\n", "initial");

    let status = repo.open().status().unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(!status.detached);
    assert_eq!(status.head.as_deref(), Some(head.as_str()));
    assert!(status.is_clean());
    assert_eq!(status.upstream, None);
    // A branch with no upstream and commits on no remote has unpushed work.
    assert_eq!(status.unpushed, 1);
    assert!(status.has_uncommitted_or_unpushed());
}

#[test]
fn status_repo_without_commits() {
    let repo = TestRepo::init();
    let status = repo.open().status().unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.head, None);
    assert!(status.is_clean());
    assert!(!status.has_uncommitted_or_unpushed());
}

#[test]
fn status_tracked_modification_only() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    repo.write("a.txt", "changed\n");

    let s = repo.open().status().unwrap();
    assert_eq!((s.modified, s.staged, s.untracked), (1, 0, 0));
    assert!(!s.is_clean());
}

#[test]
fn status_staged_only() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    repo.write("a.txt", "changed\n");
    repo.git(&["add", "a.txt"]);

    let s = repo.open().status().unwrap();
    assert_eq!((s.modified, s.staged, s.untracked), (0, 1, 0));
}

#[test]
fn status_staged_and_further_modified() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);
    repo.write("a.txt", "three\n");

    let s = repo.open().status().unwrap();
    assert_eq!((s.modified, s.staged, s.untracked), (1, 1, 0));
}

#[test]
fn status_untracked_only() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    repo.write("dir/new.txt", "x\n");
    repo.write("other.txt", "y\n");

    let s = repo.open().status().unwrap();
    assert_eq!((s.modified, s.staged, s.untracked), (0, 0, 2));
    assert!(!s.is_clean());
}

#[test]
fn status_ignores_ignored_files() {
    let repo = TestRepo::init();
    repo.commit(".gitignore", "*.log\n", "ignore");
    repo.write("debug.log", "noise\n");
    assert!(repo.open().status().unwrap().is_clean());
}

#[test]
fn status_deleted_tracked_file_counts_as_modified() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    std::fs::remove_file(repo.path().join("a.txt")).unwrap();
    let s = repo.open().status().unwrap();
    assert_eq!((s.modified, s.staged, s.untracked), (1, 0, 0));
}

#[test]
fn status_detached_head() {
    let repo = TestRepo::init();
    let first = repo.commit("a.txt", "one\n", "initial");
    repo.commit("a.txt", "two\n", "second");
    repo.git(&["checkout", "--detach", &first]);

    let s = repo.open().status().unwrap();
    assert!(s.detached);
    assert_eq!(s.branch, None);
    assert_eq!(s.head.as_deref(), Some(first.as_str()));
    assert!(s.head_label().starts_with("detached at "));
}

#[test]
fn status_in_sync_with_upstream() {
    let (work, _origin) = TestRepo::with_origin();
    let s = work.open().status().unwrap();
    assert_eq!(s.upstream.as_deref(), Some("origin/main"));
    assert_eq!((s.ahead, s.behind, s.unpushed), (0, 0, 0));
    assert!(!s.has_uncommitted_or_unpushed());
}

#[test]
fn status_ahead_of_upstream() {
    let (work, _origin) = TestRepo::with_origin();
    work.commit("b.txt", "b\n", "local");

    let s = work.open().status().unwrap();
    assert_eq!((s.ahead, s.behind, s.unpushed), (1, 0, 1));
    assert!(s.is_clean());
    assert!(s.has_uncommitted_or_unpushed());
}

#[test]
fn status_behind_upstream() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    other.commit("b.txt", "b\n", "remote change");
    other.git(&["push", "origin", "main"]);
    work.open().fetch("origin").unwrap();

    let s = work.open().status().unwrap();
    assert_eq!((s.ahead, s.behind, s.unpushed), (0, 1, 0));
    // Being behind is not something that can be lost.
    assert!(!s.has_uncommitted_or_unpushed());
}

#[test]
fn status_diverged_from_upstream() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    other.commit("b.txt", "b\n", "remote change");
    other.git(&["push", "origin", "main"]);
    work.commit("c.txt", "c\n", "local change");
    work.open().fetch("origin").unwrap();

    let s = work.open().status().unwrap();
    assert_eq!((s.ahead, s.behind, s.unpushed), (1, 1, 1));
}

#[test]
fn status_new_branch_without_upstream_and_no_new_commits_is_not_unpushed() {
    let (work, _origin) = TestRepo::with_origin();
    work.git(&["switch", "-c", "feature/x"]);

    let s = work.open().status().unwrap();
    assert_eq!(s.upstream, None);
    assert_eq!(s.unpushed, 0);
    assert!(!s.has_uncommitted_or_unpushed());

    work.commit("f.txt", "f\n", "feature work");
    let s = work.open().status().unwrap();
    assert_eq!(s.unpushed, 1);
    assert!(s.has_uncommitted_or_unpushed());
}

// ------------------------------------------------------ branches and keys

#[test]
fn key_matching_on_real_branches() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    for name in [
        "feature/PROJ-123-x",
        "feature/PROJ-1234-other",
        "feature/PROJ-12-shorter",
        "feature/proj-123-add-x",
        "PROJ-123/short",
        "hotfix/PROJ-123_x",
        "XPROJ-123-foo",
        "unrelated",
    ] {
        repo.git(&["branch", name]);
    }
    let git = repo.open();

    let mut found = names(&git.find_branches_for_key("PROJ-123").unwrap())
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    found.sort();
    assert_eq!(
        found,
        [
            "PROJ-123/short",
            "feature/PROJ-123-x",
            "feature/proj-123-add-x",
            "hotfix/PROJ-123_x",
        ]
    );

    // PROJ-12 must not pick up PROJ-123 or PROJ-1234.
    let short = git.find_branches_for_key("PROJ-12").unwrap();
    assert_eq!(names(&short), ["feature/PROJ-12-shorter"]);
    assert!(!short[0].duplicate);

    // Key given in another case.
    assert_eq!(git.find_branches_for_key("proj-1234").unwrap().len(), 1);
    assert!(git.find_branches_for_key("NOPE-1").unwrap().is_empty());
}

#[test]
fn duplicate_flag_is_set_only_when_several_branches_match() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    repo.git(&["branch", "feature/PROJ-7-first"]);
    let git = repo.open();
    let one = git.find_branches_for_key("PROJ-7").unwrap();
    assert_eq!(one.len(), 1);
    assert!(!one[0].duplicate);

    repo.git(&["branch", "feature/PROJ-7-second"]);
    let two = git.find_branches_for_key("PROJ-7").unwrap();
    assert_eq!(two.len(), 2);
    assert!(two.iter().all(|m| m.duplicate));
}

#[test]
fn matches_are_ordered_local_first_then_newest() {
    let (work, origin) = TestRepo::with_origin();
    let base = work.branch_sha("main");

    // Local branches with distinct commit times.
    work.git(&["switch", "-c", "feature/PROJ-9-old"]);
    work.commit_at("old.txt", "o\n", "old", 1_000_000_000);
    work.git(&["switch", "main"]);
    work.git(&["switch", "-c", "feature/PROJ-9-new"]);
    work.commit_at("new.txt", "n\n", "new", 1_600_000_000);
    work.git(&["switch", "main"]);

    // A remote-only branch, newer than both.
    let other = TestRepo::clone_from(origin.path());
    other.git(&["switch", "-c", "feature/PROJ-9-remote", &base]);
    other.commit_at("r.txt", "r\n", "remote", 1_700_000_000);
    other.git(&["push", "origin", "feature/PROJ-9-remote"]);
    work.open().fetch("origin").unwrap();

    let matches = work.open().find_branches_for_key("PROJ-9").unwrap();
    assert_eq!(
        names(&matches),
        [
            "feature/PROJ-9-new",
            "feature/PROJ-9-old",
            "feature/PROJ-9-remote"
        ]
    );
    assert!(matches[0].branch.is_local());
    assert!(!matches[2].branch.is_local());
    assert!(matches.iter().all(|m| m.duplicate));
}

#[test]
fn local_and_remote_refs_fold_into_one_logical_branch() {
    let (work, origin) = TestRepo::with_origin();
    work.git(&["switch", "-c", "feature/PROJ-5-a"]);
    work.commit("f.txt", "f\n", "feature");
    work.git(&["push", "-u", "origin", "feature/PROJ-5-a"]);

    // A remote-only branch made by someone else.
    let other = TestRepo::clone_from(origin.path());
    other.git(&["switch", "-c", "feature/PROJ-5-b"]);
    other.commit("g.txt", "g\n", "other");
    other.git(&["push", "origin", "feature/PROJ-5-b"]);
    work.open().fetch("origin").unwrap();

    let git = work.open();
    let flat = git.branches().unwrap();
    let flat_a: Vec<_> = flat
        .iter()
        .filter(|b| b.name == "feature/PROJ-5-a")
        .collect();
    assert_eq!(flat_a.len(), 2, "both refs stay available");
    // origin/HEAD is not a branch.
    assert!(flat.iter().all(|b| b.name != "HEAD"));

    let local = flat_a.iter().find(|b| b.kind == BranchKind::Local).unwrap();
    assert_eq!(local.upstream.as_deref(), Some("origin/feature/PROJ-5-a"));
    let remote = flat_a
        .iter()
        .find(|b| b.kind == BranchKind::Remote)
        .unwrap();
    assert_eq!(remote.remote.as_deref(), Some("origin"));
    assert_eq!(remote.refname, "origin/feature/PROJ-5-a");
    assert!(remote.tracked_by_local);
    assert_eq!(local.tip, remote.tip);

    let matches = git.find_branches_for_key("PROJ-5").unwrap();
    assert_eq!(matches.len(), 2, "two logical branches, not three refs");
    let a = matches
        .iter()
        .find(|m| m.branch.name == "feature/PROJ-5-a")
        .unwrap();
    assert!(a.branch.local.is_some());
    assert_eq!(a.branch.remotes.len(), 1);
    let b = matches
        .iter()
        .find(|m| m.branch.name == "feature/PROJ-5-b")
        .unwrap();
    assert!(b.branch.local.is_none());
    assert_eq!(b.branch.remotes.len(), 1);
    assert!(!b.branch.remotes[0].tracked_by_local);
}

// ------------------------------------------------------------ resolve_base

const PREFERENCE: [&str; 3] = ["develop", "main", "master"];

#[test]
fn resolve_base_only_main() {
    let repo = TestRepo::init_with_branch("main");
    repo.commit("a.txt", "x\n", "initial");
    let base = repo.open().resolve_base(&PREFERENCE).unwrap().unwrap();
    assert_eq!(base.name, "main");
    assert!(base.local);
    assert_eq!(base.refname(), "main");
}

#[test]
fn resolve_base_only_master() {
    let repo = TestRepo::init_with_branch("master");
    repo.commit("a.txt", "x\n", "initial");
    assert_eq!(
        repo.open().default_branch(&PREFERENCE).unwrap().as_deref(),
        Some("master")
    );
}

#[test]
fn resolve_base_follows_the_preference_order() {
    let repo = TestRepo::init_with_branch("main");
    repo.commit("a.txt", "x\n", "initial");
    repo.git(&["branch", "master"]);
    let git = repo.open();
    assert_eq!(
        git.default_branch(&["develop", "main", "master"])
            .unwrap()
            .as_deref(),
        Some("main")
    );
    assert_eq!(
        git.default_branch(&["master", "main"]).unwrap().as_deref(),
        Some("master")
    );

    repo.git(&["branch", "develop"]);
    assert_eq!(
        git.default_branch(&PREFERENCE).unwrap().as_deref(),
        Some("develop")
    );
}

#[test]
fn resolve_base_neither_exists() {
    let repo = TestRepo::init_with_branch("trunk");
    repo.commit("a.txt", "x\n", "initial");
    assert_eq!(repo.open().resolve_base(&PREFERENCE).unwrap(), None);
}

#[test]
fn resolve_base_finds_remote_only_branch() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    other.git(&["switch", "-c", "develop"]);
    other.commit("d.txt", "d\n", "develop");
    other.git(&["push", "origin", "develop"]);
    work.open().fetch("origin").unwrap();

    let base = work.open().resolve_base(&PREFERENCE).unwrap().unwrap();
    assert_eq!(base.name, "develop");
    assert!(!base.local);
    assert_eq!(base.remote_refs, ["origin/develop"]);
    assert_eq!(base.refname(), "origin/develop");
}

// ------------------------------------------------------------------- diff

fn file<'a>(diff: &'a RepoDiff, path: &str) -> &'a FileDiff {
    diff.files
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| {
            panic!(
                "{path} not in diff: {:?}",
                diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
            )
        })
}

#[test]
fn three_dot_diff_ignores_changes_made_on_base_after_branching() {
    let repo = TestRepo::init();
    let lines = |x: &str| format!("l1\nl2\nl3\n{x}\nl5\nl6\nl7\nl8\n");
    repo.commit("a.txt", &lines("l4"), "a");
    repo.write("b.txt", "b\n");
    repo.write("del.txt", "gone\n");
    repo.write(
        "old.txt",
        "some\nfairly\nlong\ncontent\nhere\nfor\nrename\ndetection\n",
    );
    repo.write_bytes("bin.dat", &[0, 1, 2, 3, 0, 255]);
    repo.write("c.txt", "c1\nc2\nc3\n");
    repo.commit_all("base files");
    let fork = repo.branch_sha("main");

    // Feature work.
    repo.git(&["switch", "-c", "feature"]);
    repo.write("a.txt", &lines("CHANGED"));
    repo.write("new.txt", "brand\nnew\n");
    std::fs::remove_file(repo.path().join("del.txt")).unwrap();
    repo.git(&["mv", "old.txt", "renamed.txt"]);
    repo.write_bytes("bin.dat", &[9, 0, 8, 0, 7]);
    repo.write("c.txt", "top1\ntop2\nc1\nc2\nc3\n");
    repo.commit_all("feature work");

    // Base moves on after the branch point.
    repo.git(&["switch", "main"]);
    repo.write("b.txt", "b changed on main\n");
    repo.write("main-only.txt", "m\n");
    repo.commit_all("main moves on");

    let diff = repo.open().diff("main", "feature").unwrap();
    assert_eq!(diff.merge_base, fork);

    let mut paths: Vec<_> = diff.files.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "a.txt",
            "bin.dat",
            "c.txt",
            "del.txt",
            "new.txt",
            "renamed.txt"
        ]
    );

    // Modified file with exact hunk numbers.
    let a = file(&diff, "a.txt");
    assert_eq!(a.status, FileStatus::Modified);
    assert_eq!((a.additions, a.deletions), (1, 1));
    assert_eq!(a.hunks.len(), 1);
    let hunk = &a.hunks[0];
    assert_eq!(
        (
            hunk.old_start,
            hunk.old_lines,
            hunk.new_start,
            hunk.new_lines
        ),
        (1, 7, 1, 7)
    );
    let removed = hunk
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Removed)
        .unwrap();
    assert_eq!(
        (
            removed.old_lineno,
            removed.new_lineno,
            removed.content.as_str()
        ),
        (Some(4), None, "l4")
    );
    let added = hunk
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Added)
        .unwrap();
    assert_eq!(
        (added.old_lineno, added.new_lineno, added.content.as_str()),
        (None, Some(4), "CHANGED")
    );
    let first_ctx = &hunk.lines[0];
    assert_eq!(first_ctx.kind, LineKind::Context);
    assert_eq!(
        (first_ctx.old_lineno, first_ctx.new_lineno),
        (Some(1), Some(1))
    );

    // Lines inserted at the top shift the new-side numbering.
    let c = file(&diff, "c.txt");
    assert_eq!((c.additions, c.deletions), (2, 0));
    let inserted: Vec<_> = c.hunks[0]
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Added)
        .collect();
    assert_eq!(
        inserted.iter().map(|l| l.new_lineno).collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    let ctx = c.hunks[0]
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Context)
        .unwrap();
    assert_eq!((ctx.old_lineno, ctx.new_lineno), (Some(1), Some(3)));

    // Added, deleted, renamed, binary.
    let new = file(&diff, "new.txt");
    assert_eq!(
        (new.status, new.additions, new.deletions),
        (FileStatus::Added, 2, 0)
    );
    let del = file(&diff, "del.txt");
    assert_eq!(
        (del.status, del.additions, del.deletions),
        (FileStatus::Deleted, 0, 1)
    );
    assert_eq!(del.hunks[0].lines[0].content, "gone");
    let renamed = file(&diff, "renamed.txt");
    assert_eq!(renamed.status, FileStatus::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("old.txt"));
    assert_eq!(renamed.hunks.len(), 0);
    let bin = file(&diff, "bin.dat");
    assert!(bin.binary);
    assert!(bin.hunks.is_empty());
    assert!(!a.binary);

    assert_eq!(diff.additions(), 1 + 2 + 2);
    assert_eq!(diff.deletions(), 1 + 1);
}

#[test]
fn diff_does_not_touch_the_working_tree() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("f.txt", "f\n", "feature");
    repo.git(&["switch", "main"]);
    repo.write("scratch.txt", "uncommitted\n");

    let diff = repo.open().diff("main", "feature").unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert!(!repo.exists("f.txt"));
    assert!(repo.exists("scratch.txt"));
}

#[test]
fn diff_works_between_remote_tracking_refs() {
    let (work, origin) = TestRepo::with_origin();
    work.git(&["switch", "-c", "develop"]);
    work.git(&["push", "-u", "origin", "develop"]);

    let other = TestRepo::clone_from(origin.path());
    other.git(&["switch", "-c", "feature/x", "origin/develop"]);
    other.commit("x.txt", "x\n", "feature");
    other.git(&["push", "origin", "feature/x"]);
    other.git(&["switch", "develop"]);
    other.commit("d.txt", "d\n", "develop moves");
    other.git(&["push", "origin", "develop"]);

    // No local `feature/x` exists here, and local develop is stale.
    work.open().fetch("origin").unwrap();
    let diff = work
        .open()
        .diff("origin/develop", "origin/feature/x")
        .unwrap();
    let paths: Vec<_> = diff.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["x.txt"]);
}

#[test]
fn diff_of_unknown_revision_is_an_error() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "one\n", "initial");
    let err = repo.open().diff("main", "nope").unwrap_err();
    assert!(format!("{err:#}").contains("nope"));
}

// ----------------------------------------------------------------- history

#[test]
fn merge_base_ancestry_and_commits_not_in() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "1\n", "c1");
    let c2 = repo.commit("a.txt", "2\n", "c2");
    repo.git(&["switch", "-c", "feat"]);
    repo.commit("f.txt", "1\n", "f1");
    repo.commit("f.txt", "2\n", "f2");
    repo.git(&["switch", "main"]);
    repo.commit("a.txt", "3\n", "c3");
    let git = repo.open();

    assert_eq!(
        git.merge_base("main", "feat").unwrap().as_deref(),
        Some(c2.as_str())
    );
    assert!(git.is_ancestor(&c2, "feat").unwrap());
    assert!(git.is_ancestor("main", "main").unwrap());
    assert!(!git.is_ancestor("feat", "main").unwrap());
    assert!(!git.is_ancestor("main", "feat").unwrap());

    let ahead = git.commits_not_in("main", "feat").unwrap();
    assert_eq!(
        ahead.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>(),
        ["f2", "f1"]
    );
    assert_eq!(ahead[0].author, "Test User");
    let behind = git.commits_not_in("feat", "main").unwrap();
    assert_eq!(
        behind
            .iter()
            .map(|c| c.summary.as_str())
            .collect::<Vec<_>>(),
        ["c3"]
    );
    assert!(git.commits_not_in("main", "main").unwrap().is_empty());
}

#[test]
fn merge_base_of_unrelated_histories_is_none() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "1\n", "c1");
    repo.git(&["switch", "--orphan", "island"]);
    repo.commit("i.txt", "i\n", "island");
    let git = repo.open();
    assert_eq!(git.merge_base("main", "island").unwrap(), None);
    assert!(git.diff("main", "island").is_err());
}

// ------------------------------------------------------------ fetch and ff

#[test]
fn fetch_reports_command_and_stderr_on_failure() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "1\n", "c1");
    let err = repo.open().fetch("nowhere").unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains("nowhere"), "{message}");
    assert!(message.contains("git fetch"), "{message}");
}

#[test]
fn fast_forward_current_branch() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    let pushed = other.commit("b.txt", "b\n", "remote change");
    other.git(&["push", "origin", "main"]);

    let git = work.open();
    // Not yet fetched: nothing to do.
    assert_eq!(git.fast_forward("main").unwrap(), FastForward::UpToDate);

    git.fetch("origin").unwrap();
    let before = work.branch_sha("main");
    assert_eq!(
        git.fast_forward("main").unwrap(),
        FastForward::Advanced {
            from: before,
            to: pushed.clone()
        }
    );
    assert_eq!(work.branch_sha("HEAD"), pushed);
    assert!(work.exists("b.txt"));
    assert_eq!(git.fast_forward("main").unwrap(), FastForward::UpToDate);
}

#[test]
fn fast_forward_a_branch_that_is_not_checked_out() {
    let (work, origin) = TestRepo::with_origin();
    work.git(&["branch", "other-work"]);
    work.git(&["switch", "other-work"]);
    let other = TestRepo::clone_from(origin.path());
    let pushed = other.commit("b.txt", "b\n", "remote change");
    other.git(&["push", "origin", "main"]);
    work.open().fetch("origin").unwrap();

    let outcome = work.open().fast_forward("main").unwrap();
    assert!(matches!(outcome, FastForward::Advanced { .. }));
    assert_eq!(work.branch_sha("main"), pushed);
    // The checkout did not move.
    assert_eq!(
        work.git(&["rev-parse", "--abbrev-ref", "HEAD"]),
        "other-work"
    );
    assert!(!work.exists("b.txt"));
}

#[test]
fn fast_forward_refuses_diverged_history_and_changes_nothing() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    other.commit("b.txt", "b\n", "remote change");
    other.git(&["push", "origin", "main"]);
    let local = work.commit("c.txt", "c\n", "local change");
    work.open().fetch("origin").unwrap();

    let outcome = work.open().fast_forward("main").unwrap();
    assert_eq!(
        outcome,
        FastForward::Diverged {
            ahead: 1,
            behind: 1
        }
    );
    assert_eq!(work.branch_sha("main"), local);
    assert_eq!(work.branch_sha("HEAD"), local);
    assert!(work.exists("c.txt"));
    assert!(!work.exists("b.txt"));
    assert!(work.open().status().unwrap().is_clean());

    // Same when the diverged branch is not the one checked out.
    work.git(&["switch", "-c", "elsewhere"]);
    assert_eq!(
        work.open().fast_forward("main").unwrap(),
        FastForward::Diverged {
            ahead: 1,
            behind: 1
        }
    );
    assert_eq!(work.branch_sha("main"), local);
}

#[test]
fn fast_forward_without_upstream() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "1\n", "c1");
    assert_eq!(
        repo.open().fast_forward("main").unwrap(),
        FastForward::NoUpstream
    );
    assert!(repo.open().fast_forward("missing").is_err());
}

// ------------------------------------------------------------------ stash

#[test]
fn stash_push_returns_none_when_clean() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "1\n", "c1");
    assert_eq!(repo.open().stash_push("nothing").unwrap(), None);
    assert!(repo.open().stash_list().unwrap().is_empty());
}

#[test]
fn stash_pop_finds_the_right_stash_among_several() {
    let repo = TestRepo::init();
    repo.write("a.txt", "a0\n");
    repo.write("b.txt", "b0\n");
    repo.commit_all("initial");
    let git = repo.open();

    repo.write("a.txt", "a-stashed\n");
    let first = git.stash_push("task:one").unwrap().unwrap();
    repo.write("b.txt", "b-stashed\n");
    let second = git.stash_push("task:two").unwrap().unwrap();
    repo.write("untracked.txt", "new file\n");
    let third = git.stash_push("task:three").unwrap().unwrap();

    // Untracked files were stashed too, and the tree is clean.
    assert!(!repo.exists("untracked.txt"));
    assert!(git.status().unwrap().is_clean());
    assert_eq!(git.stash_list().unwrap().len(), 3);
    assert_ne!(first.commit, second.commit);

    // Pop the oldest one, which sits at the bottom of the stack (stash@{2}).
    git.stash_pop(&first).unwrap();
    assert_eq!(repo.read("a.txt"), "a-stashed\n");
    assert_eq!(repo.read("b.txt"), "b0\n");
    assert!(!repo.exists("untracked.txt"));
    let remaining = git.stash_list().unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(remaining[0].message.ends_with("task:three"));
    assert!(remaining[1].message.ends_with("task:two"));

    // Indexes shifted; the untracked-files stash is now stash@{0}.
    git.stash_pop(&third).unwrap();
    assert_eq!(repo.read("untracked.txt"), "new file\n");
    assert_eq!(repo.read("b.txt"), "b0\n");

    git.stash_pop(&second).unwrap();
    assert_eq!(repo.read("b.txt"), "b-stashed\n");
    assert!(git.stash_list().unwrap().is_empty());

    // Already popped.
    assert!(git.stash_pop(&second).is_err());
}

#[test]
fn stash_pop_falls_back_to_label_when_commit_is_unknown() {
    let repo = TestRepo::init();
    repo.write("a.txt", "a0\n");
    repo.write("b.txt", "b0\n");
    repo.commit_all("initial");
    let git = repo.open();

    repo.write("a.txt", "A\n");
    git.stash_push("label-a").unwrap().unwrap();
    repo.write("b.txt", "B\n");
    git.stash_push("label-b").unwrap().unwrap();

    let stale = StashRef {
        label: "label-a".into(),
        commit: "0".repeat(40),
    };
    git.stash_pop(&stale).unwrap();
    assert_eq!(repo.read("a.txt"), "A\n");
    assert_eq!(repo.read("b.txt"), "b0\n");
    assert_eq!(git.stash_list().unwrap().len(), 1);
}

#[test]
fn stash_pop_conflict_keeps_the_stash() {
    let repo = TestRepo::init();
    repo.commit("a.txt", "a0\n", "initial");
    let git = repo.open();
    repo.write("a.txt", "stashed\n");
    let stash = git.stash_push("conflicting").unwrap().unwrap();
    repo.commit("a.txt", "committed meanwhile\n", "diverge");

    let err = git.stash_pop(&stash).unwrap_err();
    assert!(format!("{err:#}").contains("conflicting"));
    assert_eq!(
        git.stash_list().unwrap().len(),
        1,
        "stash is kept for the user"
    );
}

// ----------------------------------------------------------------- switch

fn repo_with_feature_branch() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("a.txt", "a0\n");
    repo.commit_all("initial");
    repo.git(&["switch", "-c", "feature"]);
    repo.commit("f.txt", "f\n", "feature");
    repo.git(&["switch", "main"]);
    repo
}

#[test]
fn switch_clean_tree() {
    let repo = repo_with_feature_branch();
    let outcome = repo.open().switch("feature", OnDirty::Stash).unwrap();
    assert_eq!(outcome.previous_branch.as_deref(), Some("main"));
    assert_eq!(outcome.stash, None);
    assert!(!outcome.created_tracking_branch);
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "feature");

    let again = repo.open().switch("feature", OnDirty::Stash).unwrap();
    assert!(again.already_on_branch);
}

#[test]
fn switch_dirty_tree_stashes_then_restores() {
    let repo = repo_with_feature_branch();
    repo.write("a.txt", "modified\n");
    repo.write("scratch.txt", "untracked\n");
    let git = repo.open();

    let outcome = git.switch("feature", OnDirty::default()).unwrap();
    let stash = outcome.stash.clone().expect("dirty tree was stashed");
    assert_eq!(outcome.previous_branch.as_deref(), Some("main"));
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "feature");
    assert!(git.status().unwrap().is_clean());
    assert!(!repo.exists("scratch.txt"));

    // Restore the way a caller (deactivate) would: back to the branch, then pop.
    git.switch(outcome.previous_branch.as_deref().unwrap(), OnDirty::Abort)
        .unwrap();
    git.stash_pop(&stash).unwrap();
    assert_eq!(repo.read("a.txt"), "modified\n");
    assert_eq!(repo.read("scratch.txt"), "untracked\n");
    assert!(git.stash_list().unwrap().is_empty());
}

#[test]
fn switch_uses_a_caller_supplied_stash_label() {
    let repo = repo_with_feature_branch();
    repo.write("a.txt", "modified\n");
    let git = repo.open();
    let outcome = git
        .switch("feature", OnDirty::StashLabelled("PROJ-1 parked".into()))
        .unwrap();
    assert_eq!(outcome.stash.unwrap().label, "PROJ-1 parked");
    assert!(
        git.stash_list().unwrap()[0]
            .message
            .ends_with("PROJ-1 parked")
    );
}

#[test]
fn switch_dirty_tree_with_abort_changes_nothing() {
    let repo = repo_with_feature_branch();
    repo.write("a.txt", "modified\n");
    let git = repo.open();

    let err = git.switch("feature", OnDirty::Abort).unwrap_err();
    assert!(format!("{err:#}").contains("uncommitted"));
    assert_eq!(repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(repo.read("a.txt"), "modified\n");
    assert!(git.stash_list().unwrap().is_empty());
}

#[test]
fn switch_to_missing_branch_does_not_stash() {
    let repo = repo_with_feature_branch();
    repo.write("a.txt", "modified\n");
    let git = repo.open();

    assert!(git.switch("nope", OnDirty::Stash).is_err());
    assert!(git.stash_list().unwrap().is_empty());
    assert_eq!(repo.read("a.txt"), "modified\n");
}

#[test]
fn switch_to_remote_only_branch_creates_tracking_branch() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    other.git(&["switch", "-c", "feature/PROJ-8-y"]);
    other.commit("y.txt", "y\n", "remote feature");
    other.git(&["push", "origin", "feature/PROJ-8-y"]);
    work.open().fetch("origin").unwrap();
    work.write("a.txt", "dirty\n");

    let git = work.open();
    let outcome = git.switch("feature/PROJ-8-y", OnDirty::Stash).unwrap();
    assert!(outcome.created_tracking_branch);
    assert!(outcome.stash.is_some());
    assert!(work.exists("y.txt"));

    let status = git.status().unwrap();
    assert_eq!(status.branch.as_deref(), Some("feature/PROJ-8-y"));
    assert_eq!(status.upstream.as_deref(), Some("origin/feature/PROJ-8-y"));
    assert_eq!((status.ahead, status.behind), (0, 0));
}

// -------------------------------------------------------------- worktrees

/// `main` plus `uat` (at main) and two feature branches touching the same file.
fn integration_repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("shared.txt", "base\n");
    repo.commit_all("initial");
    repo.git(&["branch", "uat"]);
    for (name, content) in [("feat-a", "from a\n"), ("feat-b", "from b\n")] {
        repo.git(&["switch", "-c", name, "main"]);
        repo.commit("shared.txt", content, name);
    }
    repo.git(&["switch", "-c", "feat-c", "main"]);
    repo.commit("c.txt", "c\n", "feat-c");
    repo.git(&["switch", "main"]);
    repo
}

#[test]
fn worktree_merge_clean_and_up_to_date() {
    let repo = integration_repo();
    let scratch = TempDir::new().unwrap();
    let wt = fresh_path(&scratch, "uat-wt");
    let git = repo.open();

    git.worktree_add(&wt, "uat").unwrap();
    let merged = git.merge_in_worktree(&wt, "feat-c").unwrap();
    let MergeOutcome::Merged { commit } = merged else {
        panic!("expected a merge, got {merged:?}");
    };
    assert_eq!(commit, repo.branch_sha("uat"));
    assert!(wt.join("c.txt").exists());

    // Merge commit with both parents.
    assert_eq!(
        repo.git(&["rev-list", "--parents", "-n", "1", &commit])
            .split(' ')
            .count(),
        3
    );

    // The main checkout is untouched.
    assert!(!repo.exists("c.txt"));
    assert!(git.status().unwrap().is_clean());
    assert_eq!(git.status().unwrap().branch.as_deref(), Some("main"));

    assert_eq!(
        git.merge_in_worktree(&wt, "feat-c").unwrap(),
        MergeOutcome::UpToDate
    );
    assert_eq!(repo.branch_sha("uat"), commit);

    git.worktree_remove(&wt).unwrap();
    assert!(!wt.exists());
}

#[test]
fn worktree_merge_conflict_reports_files_and_leaves_everything_clean() {
    let repo = integration_repo();
    let scratch = TempDir::new().unwrap();
    let wt = fresh_path(&scratch, "uat-wt");
    let git = repo.open();

    git.worktree_add(&wt, "uat").unwrap();
    let MergeOutcome::Merged { commit } = git.merge_in_worktree(&wt, "feat-a").unwrap() else {
        panic!("first merge should succeed");
    };

    let outcome = git.merge_in_worktree(&wt, "feat-b").unwrap();
    assert_eq!(
        outcome,
        MergeOutcome::Conflict {
            files: vec!["shared.txt".into()]
        }
    );

    let wt_status = GitRepo::open(&wt).unwrap().status().unwrap();
    assert!(wt_status.is_clean(), "worktree left dirty: {wt_status:?}");
    assert_eq!(repo.branch_sha("uat"), commit, "uat did not move");
    assert_eq!(
        std::fs::read_to_string(wt.join("shared.txt")).unwrap(),
        "from a\n"
    );
    assert!(
        !repo
            .git(&["worktree", "list", "--porcelain"])
            .contains("MERGE")
    );
    assert!(git.status().unwrap().is_clean());
    assert_eq!(repo.read("shared.txt"), "base\n");

    // Clean enough to remove without forcing.
    git.worktree_remove(&wt).unwrap();
}

#[test]
fn worktree_merge_refuses_a_dirty_worktree_and_unknown_branch() {
    let repo = integration_repo();
    let scratch = TempDir::new().unwrap();
    let wt = fresh_path(&scratch, "uat-wt");
    let git = repo.open();
    git.worktree_add(&wt, "uat").unwrap();

    let err = git.merge_in_worktree(&wt, "no-such-branch").unwrap_err();
    assert!(format!("{err:#}").contains("no-such-branch"));
    assert!(GitRepo::open(&wt).unwrap().status().unwrap().is_clean());

    std::fs::write(wt.join("dirt.txt"), "x").unwrap();
    assert!(git.merge_in_worktree(&wt, "feat-c").is_err());
    // Removing without force refuses too.
    assert!(git.worktree_remove(&wt).is_err());
}

#[test]
fn worktree_add_creates_tracking_branch_for_remote_only_branch() {
    let (work, origin) = TestRepo::with_origin();
    let other = TestRepo::clone_from(origin.path());
    other.git(&["switch", "-c", "uat"]);
    other.commit("u.txt", "u\n", "uat");
    other.git(&["push", "origin", "uat"]);
    work.open().fetch("origin").unwrap();

    let scratch = TempDir::new().unwrap();
    let wt = fresh_path(&scratch, "uat-wt");
    work.open().worktree_add(&wt, "uat").unwrap();
    assert!(wt.join("u.txt").exists());
    let status = GitRepo::open(&wt).unwrap().status().unwrap();
    assert_eq!(status.upstream.as_deref(), Some("origin/uat"));

    assert!(
        work.open()
            .worktree_add(&fresh_path(&scratch, "x"), "missing")
            .is_err()
    );
}

// ---------------------------------------------------------------- multi

#[test]
fn status_all_survives_a_bad_repo() {
    let good = TestRepo::init();
    good.commit("a.txt", "1\n", "c1");
    let plain_dir = TempDir::new().unwrap();
    let missing = PathBuf::from("/definitely/not/a/repo");
    let also_good = TestRepo::init();
    also_good.commit("a.txt", "1\n", "c1");
    also_good.write("x.txt", "dirty\n");

    let repos = vec![
        ("good", good.path().to_path_buf()),
        ("not-git", plain_dir.path().to_path_buf()),
        ("missing", missing),
        ("also-good", also_good.path().to_path_buf()),
    ];
    let results = status_all(&repos);

    assert_eq!(
        results.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        ["good", "not-git", "missing", "also-good"]
    );
    assert!(results[0].1.is_ok());
    assert!(results[1].1.is_err());
    assert!(results[2].1.is_err());
    assert_eq!(results[3].1.as_ref().unwrap().untracked, 1);
}

#[test]
fn stop_guard_data_lists_risky_repos_and_never_blocks_on_unreadable_ones() {
    let (synced, _o1) = TestRepo::with_origin();
    let (dirty, _o2) = TestRepo::with_origin();
    dirty.write("a.txt", "edited\n");
    let (ahead, _o3) = TestRepo::with_origin();
    ahead.commit("b.txt", "b\n", "unpushed");
    let (untracked, _o4) = TestRepo::with_origin();
    untracked.write("new.txt", "n\n");
    let plain_dir = TempDir::new().unwrap();

    let repos = vec![
        ("synced", synced.path().to_path_buf()),
        ("dirty", dirty.path().to_path_buf()),
        ("ahead", ahead.path().to_path_buf()),
        ("untracked", untracked.path().to_path_buf()),
        ("not-git", plain_dir.path().to_path_buf()),
        ("gone", PathBuf::from("/definitely/not/a/repo")),
    ];
    let report = assess_risks(status_all(&repos));

    assert!(!report.is_safe());
    let risky: Vec<_> = report.at_risk.iter().map(|r| r.name).collect();
    assert_eq!(risky, ["dirty", "ahead", "untracked"]);
    let unreadable: Vec<_> = report.unreadable.iter().map(|(n, _)| *n).collect();
    assert_eq!(unreadable, ["not-git", "gone"]);
    assert_eq!(report.at_risk[1].status.unpushed, 1);
    assert_eq!(report.at_risk[0].status.modified, 1);

    let safe = assess_risks(status_all(&[("synced", synced.path().to_path_buf())]));
    assert!(safe.is_safe());
    assert!(safe.unreadable.is_empty());
}

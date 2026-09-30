//! Real temporary git repositories for tests. Nothing here mocks git.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::TempDir;

use super::GitRepo;

/// Run git in `dir`, panicking with stderr on failure. Returns trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> String {
    git_env(dir, args, &[])
}

pub fn git_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs(envs.iter().copied())
        .output()
        .expect("git can be spawned");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}

fn configure(dir: &Path) {
    // Explicit so tests do not depend on the machine's git config.
    git(dir, &["config", "user.name", "Test User"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "init.defaultBranch", "main"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    git(dir, &["config", "core.hooksPath", "/dev/null"]);
}

/// A temporary non-bare repository.
pub struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    /// Empty repository on `main`.
    pub fn init() -> Self {
        Self::init_with_branch("main")
    }

    pub fn init_with_branch(branch: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        git(dir.path(), &["init", "-b", branch]);
        configure(dir.path());
        Self { dir }
    }

    /// Clone of `origin` with test identity configured.
    pub fn clone_from(origin: &Path) -> Self {
        let dir = TempDir::new().expect("temp dir");
        git(
            dir.path(),
            &["clone", origin.to_str().expect("utf-8 path"), "."],
        );
        configure(dir.path());
        Self { dir }
    }

    /// Repository with one commit (`a.txt`) pushed to a bare `origin`, which
    /// `main` tracks.
    pub fn with_origin() -> (Self, TempDir) {
        let origin = TempDir::new().expect("temp dir");
        git(origin.path(), &["init", "--bare", "-b", "main"]);

        let work = Self::init();
        work.commit("a.txt", "one\n", "initial");
        work.git(&[
            "remote",
            "add",
            "origin",
            origin.path().to_str().expect("utf-8 path"),
        ]);
        work.git(&["push", "-u", "origin", "main"]);
        (work, origin)
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn git(&self, args: &[&str]) -> String {
        git(self.path(), args)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.path().join(rel);
        fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
        fs::write(path, content).expect("write file");
    }

    pub fn write_bytes(&self, rel: &str, content: &[u8]) {
        fs::write(self.path().join(rel), content).expect("write file");
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path().join(rel)).expect("read file")
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path().join(rel).exists()
    }

    /// Write a file, commit everything, return the new commit's sha.
    pub fn commit(&self, rel: &str, content: &str, message: &str) -> String {
        self.write(rel, content);
        self.commit_all(message)
    }

    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Like [`commit`](Self::commit) with a fixed commit time (epoch seconds).
    pub fn commit_at(&self, rel: &str, content: &str, message: &str, epoch: i64) -> String {
        self.write(rel, content);
        self.git(&["add", "-A"]);
        let date = format!("{epoch} +0000");
        git_env(
            self.path(),
            &["commit", "-m", message],
            &[("GIT_COMMITTER_DATE", &date), ("GIT_AUTHOR_DATE", &date)],
        );
        self.git(&["rev-parse", "HEAD"])
    }

    pub fn branch_sha(&self, rev: &str) -> String {
        self.git(&["rev-parse", rev])
    }

    pub fn open(&self) -> GitRepo {
        GitRepo::open(self.path()).expect("open repo")
    }
}

/// A fresh sibling path inside `dir` that does not exist yet.
pub fn fresh_path(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

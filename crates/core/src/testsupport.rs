//! Shared fixtures for the overlay and activation tests: a workspace of real git repositories
//! with bare `origin`s, a real file-backed `state.db`, and a fake command runner.
//!
//! The workspace mirrors the flow in GOAL.md:
//!
//! - `api-client`: provider of the composer package `acme/api-client` (base `develop`)
//! - `web`: consumer with `composer.json` and `composer.lock`; its overlay maps the package to
//!   `api-client` and rebuilds with the task `build-ui` (base `develop`)
//! - `worker`: no overlay; `[activate] after = ["warm"]`; base `main` by config
//! - `docs`: never has the ticket branch; base `master` by config

use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

use crate::{
    activation::WorkspaceRepo,
    domain::TicketKey,
    git::{
        GitRepo,
        testutil::{configure, git},
    },
    overlay::{CommandOutput, CommandRunner, ExternalCommand},
    store::{Kind, Store},
};

/// The consumer's `composer.json` before any overlay. Deliberately not what serde would
/// print: escaped slashes, a unicode escape, and a trailing space.
pub const WEB_COMPOSER_JSON: &str = "{\n    \"name\": \"acme\\/web\",\n    \"description\": \"Caf\\u00e9 shop \",\n    \"require\": {\n        \"php\": \"^8.2\",\n        \"acme\\/api-client\": \"^2.1\",\n        \"monolog\\/monolog\": \"^3.0\"\n    },\n    \"repositories\": [\n        {\n            \"type\": \"composer\",\n            \"url\": \"https:\\/\\/packages.acme.test\"\n        }\n    ],\n    \"config\": {\n        \"sort-packages\": true\n    }\n}\n";

/// The consumer's `composer.lock` before any overlay (no final newline).
pub const WEB_COMPOSER_LOCK: &str = "{\n    \"packages\": [\n        {\n            \"name\": \"acme/api-client\",\n            \"version\": \"2.1.0\",\n            \"dist\": {\n                \"type\": \"zip\",\n                \"url\": \"https://packages.acme.test/api-client-2.1.0.zip\"\n            }\n        }\n    ],\n    \"content-hash\": \"0123456789abcdef\"\n}";

pub fn key(s: &str) -> TicketKey {
    s.parse().expect("valid ticket key")
}

/// A working tree plus its bare `origin`.
pub struct Repo {
    pub name: String,
    pub dir: PathBuf,
    pub origin: PathBuf,
}

impl Repo {
    pub fn git(&self, args: &[&str]) -> String {
        git(&self.dir, args)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.dir.join(rel);
        fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
        fs::write(path, content).expect("write file");
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.dir.join(rel)).expect("read file")
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.dir.join(rel).exists()
    }

    /// Git's hash of a file's exact bytes, so contents compare by hash.
    pub fn hash(&self, rel: &str) -> Option<String> {
        self.exists(rel)
            .then(|| self.git(&["hash-object", "--no-filters", rel]))
    }

    pub fn commit(&self, rel: &str, content: &str, message: &str) -> String {
        self.write(rel, content);
        self.commit_all(message)
    }

    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    /// The checked-out branch, or `HEAD` when detached.
    pub fn branch(&self) -> String {
        self.git(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    pub fn open(&self) -> GitRepo {
        GitRepo::open(&self.dir).expect("open repo")
    }

    pub fn stashes(&self) -> Vec<String> {
        self.open()
            .stash_list()
            .expect("stash list")
            .into_iter()
            .map(|s| s.message)
            .collect()
    }

    pub fn is_clean(&self) -> bool {
        self.open().status().expect("status").is_clean()
    }

    /// Create `branch` off the current branch with one commit, push it, and come back.
    /// With `remote_only` the local branch is deleted afterwards.
    pub fn add_branch(&self, branch: &str, file: &str, remote_only: bool) {
        let back = self.branch();
        self.git(&["switch", "-c", branch]);
        self.commit(file, &format!("{branch}\n"), &format!("work on {branch}"));
        self.git(&["push", "-u", "origin", branch]);
        self.git(&["switch", &back]);
        if remote_only {
            self.git(&["branch", "-D", branch]);
        }
    }

    /// Push a new commit to `origin/<branch>` from a scratch clone, leaving this checkout
    /// behind its upstream.
    pub fn advance_origin(&self, branch: &str, file: &str) -> String {
        let clone = TempDir::new().expect("temp dir");
        git(
            clone.path(),
            &["clone", self.origin.to_str().expect("utf-8"), "."],
        );
        configure(clone.path());
        git(clone.path(), &["switch", branch]);
        fs::write(clone.path().join(file), "from origin\n").expect("write");
        git(clone.path(), &["add", "-A"]);
        git(clone.path(), &["commit", "-m", "upstream work"]);
        git(clone.path(), &["push", "origin", branch]);
        git(clone.path(), &["rev-parse", "HEAD"])
    }
}

/// A recorded external command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub dir: PathBuf,
    pub label: String,
}

/// Stands in for composer and for task commands. It records every call; `composer update`
/// rewrites `composer.lock` and `composer install` creates one when missing (as the real
/// tools do), and a call whose label contains a `fail_on` entry fails.
#[derive(Default)]
pub struct FakeRunner {
    calls: RefCell<Vec<Call>>,
    fail_on: RefCell<Vec<String>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail_on(&self, label_part: &str) {
        self.fail_on.borrow_mut().push(label_part.into());
    }

    pub fn stop_failing(&self) {
        self.fail_on.borrow_mut().clear();
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    /// `<directory name>: <label>` per call, in order.
    pub fn log(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .map(|c| {
                format!(
                    "{}: {}",
                    c.dir.file_name().expect("dir name").to_string_lossy(),
                    c.label
                )
            })
            .collect()
    }

    pub fn clear(&self) {
        self.calls.borrow_mut().clear();
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: &ExternalCommand) -> eyre::Result<CommandOutput> {
        self.calls.borrow_mut().push(Call {
            dir: command.dir.clone(),
            label: command.label.clone(),
        });

        if self
            .fail_on
            .borrow()
            .iter()
            .any(|part| command.label.contains(part.as_str()))
        {
            return Ok(CommandOutput {
                success: false,
                code: Some(1),
                stdout: String::new(),
                stderr: format!("boom: {}", command.label),
            });
        }

        if command.program == "composer" {
            let lock = command.dir.join("composer.lock");
            match command.args.first().map(String::as_str) {
                Some("update") => {
                    let package = command.args.get(1).cloned().unwrap_or_default();
                    fs::write(&lock, format!("{{\"overlaid\": \"{package}\"}}\n"))?;
                    fs::create_dir_all(command.dir.join("vendor"))?;
                    fs::write(command.dir.join("vendor/state"), "symlinked\n")?;
                }
                Some("install") => {
                    if !lock.exists() {
                        fs::write(&lock, "{\"created-by-install\": true}\n")?;
                    }
                    fs::create_dir_all(command.dir.join("vendor"))?;
                    fs::write(command.dir.join("vendor/state"), "from lock\n")?;
                }
                _ => {}
            }
        }

        Ok(CommandOutput {
            success: true,
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// The whole test workspace.
pub struct Fixture {
    /// Keeps the directories alive for the life of the fixture.
    _root: TempDir,
    pub state_dir: PathBuf,
    pub store: Store,
    pub runner: FakeRunner,
    pub api_client: Repo,
    pub web: Repo,
    pub worker: Repo,
    pub docs: Repo,
}

const IGNORE: &str = "vendor/\nbuild.log\n";

impl Fixture {
    pub fn new() -> Self {
        let root = TempDir::new().expect("temp dir");
        let state_dir = root.path().join("state");

        let api_client = make_repo(
            root.path(),
            "api-client",
            "develop",
            &[
                (
                    "de.toml",
                    "[project]\nname = \"api-client\"\nworkspace = \"shop\"\n",
                ),
                ("src/Client.php", "<?php // client\n"),
            ],
        );
        let web = make_repo(
            root.path(),
            "web",
            "develop",
            &[
                (
                    "de.toml",
                    "[project]\nname = \"web\"\nworkspace = \"shop\"\n\n\
                     [tasks]\nbuild-ui = \"npm run build\"\nother = \"true\"\n\n\
                     [overlay.composer]\npackages = { \"acme/api-client\" = \"api-client\" }\nrebuild = [\"build-ui\"]\n",
                ),
                ("composer.json", WEB_COMPOSER_JSON),
                ("composer.lock", WEB_COMPOSER_LOCK),
                ("app.php", "<?php // web\n"),
            ],
        );
        let worker = make_repo(
            root.path(),
            "worker",
            "main",
            &[
                (
                    "de.toml",
                    "[project]\nname = \"worker\"\nworkspace = \"shop\"\n\n\
                     [tasks]\nwarm = \"make warm\"\n\n\
                     [branches]\nbase = \"main\"\n\n\
                     [activate]\nafter = [\"warm\"]\n",
                ),
                ("worker.php", "<?php // worker\n"),
            ],
        );
        let docs = make_repo(
            root.path(),
            "docs",
            "master",
            &[
                (
                    "de.toml",
                    "[project]\nname = \"docs\"\nworkspace = \"shop\"\n\n[branches]\nbase = \"master\"\nproduction = \"master\"\n",
                ),
                ("index.md", "# docs\n"),
            ],
        );

        let store = Store::open_in(&state_dir, Kind::State).expect("state store");
        Self {
            _root: root,
            state_dir,
            store,
            runner: FakeRunner::new(),
            api_client,
            web,
            worker,
            docs,
        }
    }

    /// A fixture with `PROJ-1` tracked and `feature/PROJ-1-api-change` on `api-client`.
    pub fn with_ticket() -> Self {
        let fx = Self::new();
        crate::store::tickets::claim(&fx.store, &key("PROJ-1"), 100).expect("claim");
        fx.api_client
            .add_branch("feature/PROJ-1-api-change", "src/Client.php", false);
        fx
    }

    /// The repos as the activation engine takes them. `web` comes first on purpose: a
    /// consumer listed before its provider must still be handled after it.
    pub fn repos(&self) -> Vec<WorkspaceRepo> {
        [&self.web, &self.api_client, &self.worker, &self.docs]
            .into_iter()
            .map(|r| WorkspaceRepo::load(&r.name, &r.dir).expect("load repo"))
            .collect()
    }

    pub fn all(&self) -> [&Repo; 4] {
        [&self.api_client, &self.web, &self.worker, &self.docs]
    }

    /// Drop the open database and open the file again, as a fresh process would.
    pub fn reopen_store(&mut self) {
        let fresh = Store::open_in(&self.state_dir, Kind::State).expect("reopen state store");
        self.store = fresh;
    }

    /// `(name, branch, head)` of every repo, to compare before and after.
    pub fn snapshot(&self) -> Vec<(String, String, String)> {
        self.all()
            .iter()
            .map(|r| (r.name.clone(), r.branch(), r.head()))
            .collect()
    }
}

fn make_repo(root: &Path, name: &str, base: &str, files: &[(&str, &str)]) -> Repo {
    let origin = root.join("origins").join(format!("{name}.git"));
    fs::create_dir_all(&origin).expect("mkdir origin");
    git(&origin, &["init", "--bare", "-b", base]);

    let dir = root.join("ws").join(name);
    fs::create_dir_all(&dir).expect("mkdir work");
    git(&dir, &["init", "-b", base]);
    configure(&dir);

    let repo = Repo {
        name: name.into(),
        dir,
        origin,
    };
    repo.write(".gitignore", IGNORE);
    for (rel, content) in files {
        repo.write(rel, content);
    }
    repo.commit_all("initial");
    repo.git(&[
        "remote",
        "add",
        "origin",
        repo.origin.to_str().expect("utf-8"),
    ]);
    repo.git(&["push", "-u", "origin", base]);
    repo
}

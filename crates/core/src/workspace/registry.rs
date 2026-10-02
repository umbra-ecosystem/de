//! The saved workspaces: what there is, which one is selected, and when each was last used.
//!
//! The registry is the `workspaces/` directory next to `config.toml` (one `<name>.toml` file
//! per workspace); the selected one is `[active] workspace` in `config.toml`. Everything here
//! is written so the message a user reads already says what to do next, because every surface
//! (CLI and GUI) shows these errors unchanged.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use eyre::{Context, eyre};
use thiserror::Error;

use crate::{
    config::Config, types::Slug, utils::get_project_dirs, workspace::config::WorkspaceConfig,
    workspace::Workspace,
};

/// One saved workspace, as the lists show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceListEntry {
    /// The name, which is also the file's name.
    pub name: Slug,
    /// How many projects it registers.
    pub projects: usize,
    /// When it was last opened or closed (unix seconds); `None` for one never opened.
    pub last_used: Option<i64>,
}

/// A workspace file that exists but could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableWorkspace {
    pub name: String,
    pub path: PathBuf,
    /// Plain words saying what is wrong with it.
    pub reason: String,
}

/// Everything in the registry, and the files in it that could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceList {
    /// Most recently used first; never-used last, then by name.
    pub entries: Vec<WorkspaceListEntry>,
    pub unreadable: Vec<UnreadableWorkspace>,
}

impl WorkspaceList {
    /// The list without any file that could not be read.
    pub fn names(&self) -> Vec<Slug> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }
}

/// Something that keeps a saved workspace from being opened or selected.
///
/// The wording is the user guidance: it names the command to run next.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error(
        "There is no workspace called '{name}'. Run `de workspace list` to see the saved ones, \
         or `de init` in a project folder to create it."
    )]
    NotFound { name: Slug },

    #[error(
        "The workspace '{name}' cannot be read: {reason}. Its file is {path}. \
         Fix or remove that file, then try again."
    )]
    Unreadable {
        name: String,
        path: PathBuf,
        reason: String,
    },
}

/// The directory the workspace files live in.
pub fn dir() -> eyre::Result<PathBuf> {
    Ok(get_project_dirs()?.config_local_dir().join("workspaces"))
}

/// The file of `name` inside a registry directory (the same shape as [`Workspace::path_from_name`]).
#[cfg(test)]
fn file_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.toml"))
}

/// Why a workspace file whose name and the workspace it holds disagree cannot be used.
fn mismatch_reason(inner_name: &str) -> String {
    format!(
        "it holds a workspace called '{inner_name}' while the file is named with a different \
         name; rename the file to match, or move it aside"
    )
}

fn reason_of(e: eyre::Report) -> String {
    let mut parts = e.chain().map(ToString::to_string).collect::<Vec<_>>();
    if parts.len() > 1 {
        // The last cause is the specific one (the toml error); the rest is context.
        let _ = parts.remove(0);
    }
    parts.join(": ")
}

/// Read every workspace file in `dir`, in `name.toml` order, keeping the ones that parse.
///
/// A broken file never hides the others: it goes into [`WorkspaceList::unreadable`] so the
/// caller can say which file is wrong while the rest still work.
pub fn list_in(dir: &Path) -> WorkspaceList {
    let mut list = WorkspaceList::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return list;
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();

    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let name = match Slug::from_str(&stem) {
            Ok(name) => name,
            Err(reason) => {
                list.unreadable.push(UnreadableWorkspace {
                    name: stem,
                    path,
                    reason: format!("its file name is not a valid workspace name: {reason}"),
                });
                continue;
            }
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                list.unreadable.push(UnreadableWorkspace {
                    name: stem,
                    path,
                    reason: format!("the file could not be read ({e})"),
                });
                continue;
            }
        };
        let config: WorkspaceConfig = match toml::from_str(&text) {
            Ok(config) => config,
            Err(e) => {
                list.unreadable.push(UnreadableWorkspace {
                    name: stem,
                    path,
                    reason: format!("it is not valid TOML ({e})"),
                });
                continue;
            }
        };
        if config.name.as_str() != stem {
            list.unreadable.push(UnreadableWorkspace {
                name: stem,
                path,
                reason: mismatch_reason(config.name.as_str()),
            });
            continue;
        }
        list.entries.push(WorkspaceListEntry {
            name,
            projects: config.projects.len(),
            last_used: config.last_used,
        });
    }

    sort(&mut list);
    list
}

/// Most recently used first; never used last; then by name, so the order is stable.
fn sort(list: &mut WorkspaceList) {
    // Sort by name first: the second sort is stable, so equal timestamps keep name order.
    list.entries.sort_by(|a, b| a.name.cmp(&b.name));
    list.entries.sort_by(|a, b| match (a.last_used, b.last_used) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

/// Every saved workspace, most recently used first.
pub fn list_workspaces() -> eyre::Result<WorkspaceList> {
    Ok(list_in(&dir()?))
}

/// Load one saved workspace, or say why it cannot be opened.
pub fn load_workspace(name: &Slug) -> Result<Workspace, WorkspaceError> {
    let path = Workspace::path_from_name(name).map_err(|e| WorkspaceError::Unreadable {
        name: name.to_string(),
        path: PathBuf::from("<unknown>"),
        reason: e.to_string(),
    })?;
    load_workspace_at(&path, name)
}

/// Load `name` out of a specific registry directory.
///
/// This is the seam that lets a test keep its files in a temporary directory instead of the
/// real config directory.
#[cfg(test)]
pub(crate) fn load_workspace_in(dir: &Path, name: &Slug) -> Result<Workspace, WorkspaceError> {
    load_workspace_at(&dir.join(format!("{name}.toml")), name)
}

fn load_workspace_at(path: &Path, name: &Slug) -> Result<Workspace, WorkspaceError> {
    if !path.exists() {
        return Err(WorkspaceError::NotFound { name: name.clone() });
    }
    Workspace::load_from_path(path.to_path_buf())
        .map_err(|e| WorkspaceError::Unreadable {
            name: name.to_string(),
            path: path.to_path_buf(),
            reason: reason_of(e),
        })?
        .ok_or_else(|| WorkspaceError::NotFound { name: name.clone() })
        .and_then(|workspace| {
            // The same rule as the list: a file whose name and workspace disagree is not
            // something to open or select, whatever command asked for it.
            if workspace.config().name.as_str() != name.as_str() {
                return Err(WorkspaceError::Unreadable {
                    name: name.to_string(),
                    path: path.to_path_buf(),
                    reason: mismatch_reason(workspace.config().name.as_str()),
                });
            }
            Ok(workspace)
        })
}

/// Record that a workspace was opened or closed just now: it orders the lists.
pub fn stamp_last_used(name: &Slug, now: i64) -> eyre::Result<()> {
    let mut workspace = load_workspace(name)?;
    workspace.config_mut().last_used = Some(now);
    workspace.save().wrap_err_with(|| {
        format!(
            "The workspace '{name}' could not be saved after recording when it was last used. \
             Check that the workspaces folder is writable and try again."
        )
    })
}

/// Select `name` as the active workspace: it must exist, and it becomes the most recently used.
///
/// This is the one place that writes `[active] workspace`, so a name that is not saved can
/// never end up in `config.toml`.
pub fn select_workspace(name: &Slug, now: i64) -> eyre::Result<Workspace> {
    let workspace = load_workspace(name)?;
    stamp_last_used(name, now)?;

    Config::mutate_persisted(|config| config.set_active_workspace(Some(name.clone())))
        .wrap_err_with(|| {
            format!(
                "The active workspace could not be recorded in config.toml. \
                 Check that the config folder is writable, then run `de workspace select {name}` again."
            )
        })?;
    Ok(workspace)
}

/// Forget the active workspace (the window shows the list of workspaces again).
pub fn deselect_workspace() -> eyre::Result<()> {
    Config::mutate_persisted(|config| config.set_active_workspace(None))
        .wrap_err("The active workspace could not be cleared in config.toml")?;
    Ok(())
}

/// The name in `[active] workspace`, whether or not the file still exists.
pub fn selected_name() -> eyre::Result<Option<Slug>> {
    Ok(Config::load()?.get_active_workspace().cloned())
}

/// What `[active] workspace` points at, including the case where it points at nothing.
#[derive(Debug)]
pub enum Selected {
    /// No workspace has ever been selected.
    None,
    /// The selected workspace, ready to use.
    Workspace(Workspace),
    /// `[active] workspace` names a workspace whose file is gone.
    Missing(Slug),
}

/// The selected workspace, distinguishing "none" from "selected but no longer saved".
pub fn selected_workspace() -> eyre::Result<Selected> {
    let Some(name) = selected_name()? else {
        return Ok(Selected::None);
    };
    match Workspace::load_from_name(&name) {
        Ok(Some(workspace)) => Ok(Selected::Workspace(workspace)),
        Ok(None) => Ok(Selected::Missing(name)),
        Err(e) => Err(eyre!(e)).wrap_err_with(|| {
            format!(
                "The selected workspace '{name}' could not be read. \
                 Its file may be broken; run `de workspace list` to see which files are readable."
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = file_of(dir, name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn toml_of(name: &str, projects: usize, last_used: Option<i64>) -> String {
        let mut text = format!("name = \"{name}\"\n");
        // Root keys must come before any `[table]` header, or they land inside one.
        if let Some(at) = last_used {
            text.push_str(&format!("last_used = {at}\n"));
        }
        for i in 0..projects {
            text.push_str(&format!(
                "\n[projects.p{i}]\ndir = \"/tmp/{name}-p{i}\"\n"
            ));
        }
        text
    }

    #[test]
    fn no_registry_directory_is_an_empty_list_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let list = list_in(&dir.path().join("nothing-here"));
        assert!(list.entries.is_empty() && list.unreadable.is_empty());
    }

    #[test]
    fn workspaces_are_listed_most_recently_used_first_and_never_used_last() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "old", &toml_of("old", 1, Some(100)));
        write(dir.path(), "new", &toml_of("new", 2, Some(300)));
        write(dir.path(), "mid", &toml_of("mid", 0, Some(200)));
        write(dir.path(), "fresh", &toml_of("fresh", 3, None));

        let list = list_in(dir.path());
        let names: Vec<&str> = list.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["new", "mid", "old", "fresh"]);
        assert_eq!(list.entries[1].projects, 0);
        assert_eq!(list.entries[3].last_used, None);
        assert!(list.unreadable.is_empty());
    }

    #[test]
    fn never_used_workspaces_are_ordered_by_name() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "zebra", &toml_of("zebra", 0, None));
        write(dir.path(), "alpha", &toml_of("alpha", 0, None));

        let list = list_in(dir.path());
        let names: Vec<&str> = list.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zebra"]);
    }

    #[test]
    fn a_broken_file_is_reported_and_the_others_still_list() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "good", &toml_of("good", 1, Some(10)));
        write(dir.path(), "broken", "name = 3 this is not toml ][");

        let list = list_in(dir.path());
        let names: Vec<&str> = list.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["good"]);
        assert_eq!(list.unreadable.len(), 1);
        assert_eq!(list.unreadable[0].name, "broken");
        assert!(
            list.unreadable[0].reason.contains("not valid TOML"),
            "{}",
            list.unreadable[0].reason
        );
    }

    #[test]
    fn a_file_whose_name_and_workspace_disagree_says_which_to_fix() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "onfile", &toml_of("infile", 0, None));

        let list = list_in(dir.path());
        assert!(list.entries.is_empty());
        assert_eq!(list.unreadable.len(), 1);
        assert!(
            list.unreadable[0].reason.contains("called 'infile'"),
            "{}",
            list.unreadable[0].reason
        );
        assert!(list.unreadable[0].reason.contains("rename the file to match"));
    }

    /// Loading is as strict as listing: a file whose name and workspace disagree never opens.
    #[test]
    fn loading_a_file_whose_name_and_workspace_disagree_says_which_to_fix() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "onfile", &toml_of("infile", 1, None));

        let err = load_workspace_in(dir.path(), &Slug::from_str("onfile").unwrap()).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("cannot be read"), "{text}");
        assert!(text.contains("called 'infile'"), "{text}");
        assert!(text.contains("rename the file to match"), "{text}");
        assert!(text.contains("onfile.toml"), "{text}");
    }

    /// The name the file holds is not what it is saved under, so it does not load under it.
    #[test]
    fn the_name_inside_a_mismatched_file_is_not_loadable_under_that_name() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "onfile", &toml_of("infile", 0, None));

        let err = load_workspace_in(dir.path(), &Slug::from_str("infile").unwrap()).unwrap_err();
        assert!(matches!(err, WorkspaceError::NotFound { .. }), "{err}");
    }

    #[test]
    fn a_file_that_is_not_a_slug_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Not-a-slug", "name = \"Not-a-slug\"\n");

        let list = list_in(dir.path());
        assert!(list.entries.is_empty());
        assert_eq!(list.unreadable.len(), 1);
        assert!(
            list.unreadable[0].reason.contains("not a valid workspace name"),
            "{}",
            list.unreadable[0].reason
        );
    }

    #[test]
    fn selecting_a_workspace_that_is_not_saved_says_how_to_make_one() {
        let name = Slug::from_str("ghost").unwrap();
        let err = load_workspace(&name).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("There is no workspace called 'ghost'"), "{text}");
        assert!(text.contains("de workspace list"), "{text}");
        assert!(text.contains("de init"), "{text}");
    }

    #[test]
    fn an_unreadable_workspace_error_names_the_file_to_fix() {
        let root = tempfile::tempdir().unwrap();
        let ws_dir = root.path().join("workspaces");
        std::fs::create_dir_all(&ws_dir).unwrap();
        let path = write(&ws_dir, "broken", "not toml at all [");
        let err = WorkspaceError::Unreadable {
            name: "broken".into(),
            path: path.clone(),
            reason: "it is not valid TOML (…)".into(),
        }
        .to_string();
        assert!(err.contains("cannot be read"), "{err}");
        assert!(err.contains(path.display().to_string().as_str()), "{err}");
        assert!(err.contains("Fix or remove that file"), "{err}");
    }

    /// Round-trips `last_used` through the real file shape, so an old file without it still loads.
    #[test]
    fn last_used_is_kept_with_the_workspace_and_absent_files_still_load() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shop.toml");
        std::fs::write(&path, toml_of("shop", 1, None)).unwrap();

        let mut workspace = Workspace::load_from_path(path.clone()).unwrap().unwrap();
        assert_eq!(workspace.config().last_used, None);
        workspace.config_mut().last_used = Some(1_700_000_000);
        workspace.save().unwrap();

        let again = Workspace::load_from_path(path).unwrap().unwrap();
        assert_eq!(again.config().last_used, Some(1_700_000_000));
        assert_eq!(again.config().projects.len(), 1);
    }
}

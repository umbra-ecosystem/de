//! Diffs read from git objects, never from the working tree.

use eyre::{Context, eyre};
use git2::{Delta, DiffFindOptions, DiffOptions, Patch};

use super::repo::GitRepo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Line number in the old file; `None` for added lines.
    pub old_lineno: Option<u32>,
    /// Line number in the new file; `None` for removed lines.
    pub new_lineno: Option<u32>,
    /// Line text without the trailing newline.
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    /// The previous path for renames.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoDiff {
    pub base: String,
    pub head: String,
    /// The commit the diff starts from: the merge base of `base` and `head`.
    pub merge_base: String,
    pub files: Vec<FileDiff>,
}

impl RepoDiff {
    pub fn additions(&self) -> usize {
        self.files.iter().map(|f| f.additions).sum()
    }

    pub fn deletions(&self) -> usize {
        self.files.iter().map(|f| f.deletions).sum()
    }
}

impl GitRepo {
    /// What a pull request from `head` into `base` shows: the changes from the
    /// merge base of the two to `head` (three-dot semantics). Changes made on
    /// `base` after `head` branched off do not appear.
    ///
    /// Works from objects only, so nothing is checked out, and accepts any
    /// revision including remote-tracking refs such as `origin/feature/x`.
    pub fn diff(&self, base: &str, head: &str) -> eyre::Result<RepoDiff> {
        let base_commit = self.commit(base)?;
        let head_commit = self.commit(head)?;

        let merge_base = self
            .inner()
            .merge_base(base_commit.id(), head_commit.id())
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("'{base}' and '{head}' have no common ancestor"))?;
        let merge_base_tree = self.inner().find_commit(merge_base)?.tree()?;
        let head_tree = head_commit.tree()?;

        let mut opts = DiffOptions::new();
        opts.context_lines(3);
        let mut diff = self
            .inner()
            .diff_tree_to_tree(Some(&merge_base_tree), Some(&head_tree), Some(&mut opts))
            .wrap_err("Failed to compute diff")?;
        diff.find_similar(Some(DiffFindOptions::new().renames(true)))
            .wrap_err("Failed to detect renames")?;

        let files = files_of(&diff)?;

        Ok(RepoDiff {
            base: base.into(),
            head: head.into(),
            merge_base: merge_base.to_string(),
            files,
        })
    }
}

impl GitRepo {
    /// What commit `sha` changed, once per parent (a root commit against the empty tree).
    ///
    /// Unlike [`diff`](Self::diff) this does no rename detection, so a file moved into place
    /// shows all of its lines as added; safety checks must see every line a commit introduces.
    pub fn commit_diffs(&self, sha: &str) -> eyre::Result<Vec<Vec<FileDiff>>> {
        let commit = self.commit(sha)?;
        let tree = commit.tree()?;
        let parents: Vec<Option<git2::Tree<'_>>> = if commit.parent_count() == 0 {
            vec![None]
        } else {
            commit
                .parents()
                .map(|p| p.tree().map(Some))
                .collect::<Result<_, _>>()?
        };

        parents
            .iter()
            .map(|parent| {
                let mut opts = DiffOptions::new();
                opts.context_lines(3);
                let diff = self
                    .inner()
                    .diff_tree_to_tree(parent.as_ref(), Some(&tree), Some(&mut opts))
                    .wrap_err("Failed to compute diff")?;
                files_of(&diff)
            })
            .collect()
    }
}

fn files_of(diff: &git2::Diff<'_>) -> eyre::Result<Vec<FileDiff>> {
    let mut files = Vec::new();
    for idx in 0..diff.deltas().len() {
        let Some(delta) = diff.get_delta(idx) else {
            continue;
        };

        let new_path = delta.new_file().path().or(delta.old_file().path());
        let path = new_path.map(|p| p.to_string_lossy().into_owned());
        let old = delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().into_owned());
        let Some(path) = path else { continue };

        let status = match delta.status() {
            Delta::Added => FileStatus::Added,
            Delta::Deleted => FileStatus::Deleted,
            Delta::Renamed => FileStatus::Renamed,
            _ => FileStatus::Modified,
        };
        let old_path = if status == FileStatus::Renamed {
            old
        } else {
            None
        };

        let mut file = FileDiff {
            path,
            old_path,
            status,
            binary: delta.flags().is_binary(),
            additions: 0,
            deletions: 0,
            hunks: Vec::new(),
        };

        if let Some(patch) = Patch::from_diff(diff, idx)? {
            file.binary = patch.delta().flags().is_binary();
            let (_, additions, deletions) = patch.line_stats()?;
            file.additions = additions;
            file.deletions = deletions;
            file.hunks = collect_hunks(&patch)?;
        }

        files.push(file);
    }
    Ok(files)
}

fn collect_hunks(patch: &Patch<'_>) -> eyre::Result<Vec<Hunk>> {
    let mut hunks = Vec::new();
    for h in 0..patch.num_hunks() {
        let (hunk, line_count) = patch.hunk(h)?;
        let mut lines = Vec::new();
        for l in 0..line_count {
            let line = patch.line_in_hunk(h, l)?;
            let kind = match line.origin() {
                ' ' => LineKind::Context,
                '+' => LineKind::Added,
                '-' => LineKind::Removed,
                // "\ No newline at end of file" markers and the like.
                _ => continue,
            };
            let content = String::from_utf8_lossy(line.content());
            lines.push(DiffLine {
                kind,
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
                content: content.trim_end_matches(['\n', '\r']).into(),
            });
        }
        hunks.push(Hunk {
            old_start: hunk.old_start(),
            old_lines: hunk.old_lines(),
            new_start: hunk.new_start(),
            new_lines: hunk.new_lines(),
            header: String::from_utf8_lossy(hunk.header()).trim_end().into(),
            lines,
        });
    }
    Ok(hunks)
}

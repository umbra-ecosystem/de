use std::path::PathBuf;

use de_core::git::{RepoStatus, status_all};

use crate::{
    types::Slug,
    utils::{get_workspace_for_cli, ui::UserInterface},
    workspace::Workspace,
};

/// One project's git state, or why it could not be read.
#[derive(Debug)]
pub struct StatusRow {
    pub project: String,
    pub status: Result<RepoStatus, String>,
}

/// `de git status`: read-only git state of every project in the workspace.
pub fn status(workspace_name: Option<Slug>) -> eyre::Result<()> {
    let ui = UserInterface::new();
    let workspace = get_workspace_for_cli(Some(workspace_name))?;

    if workspace.config().projects.is_empty() {
        ui.warning_item(
            &format!(
                "No projects found in workspace '{}'",
                workspace.config().name
            ),
            None,
        )?;
        return Ok(());
    }

    let rows = gather_status(&workspace);

    ui.heading(&format!(
        "Git status of workspace {}:",
        workspace.config().name
    ))?;
    for (i, line) in render_status_table(&rows).iter().enumerate() {
        // The first line is the column header.
        if i == 0 {
            ui.subheading(line)?;
        } else {
            ui.writeln(line)?;
        }
    }

    Ok(())
}

/// Project directories of a workspace, in name order.
pub fn project_dirs(workspace: &Workspace) -> Vec<(Slug, PathBuf)> {
    workspace
        .config()
        .projects
        .iter()
        .map(|(id, project)| (id.clone(), project.dir.clone()))
        .collect()
}

/// Read the git state of every project. A failing project only affects its own row.
pub fn gather_status(workspace: &Workspace) -> Vec<StatusRow> {
    status_all(&project_dirs(workspace))
        .into_iter()
        .map(|(name, result)| StatusRow {
            project: name.to_string(),
            status: result.map_err(|e| format!("{e:#}")),
        })
        .collect()
}

/// Lay rows out as aligned plain-text columns; the first line is the header.
pub fn render_status_table(rows: &[StatusRow]) -> Vec<String> {
    let mut cells: Vec<[String; 5]> = vec![[
        "PROJECT".into(),
        "BRANCH".into(),
        "STATE".into(),
        "AHEAD/BEHIND".into(),
        "UPSTREAM".into(),
    ]];

    // Reasons for unreadable projects go under the table so they cannot skew the columns.
    let mut notes = Vec::new();

    for row in rows {
        cells.push(match &row.status {
            Ok(status) => [
                row.project.clone(),
                status.head_label(),
                state_summary(status),
                sync_summary(status),
                upstream_summary(status),
            ],
            Err(message) => {
                notes.push(format!("! {}: {}", row.project, first_line(message)));
                [
                    row.project.clone(),
                    "-".into(),
                    "unreadable".into(),
                    "-".into(),
                    "-".into(),
                ]
            }
        });
    }

    let widths: Vec<usize> = (0..5)
        .map(|col| {
            cells
                .iter()
                .map(|c| c[col].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    let table = cells.iter().map(|row| {
        let mut line = String::new();
        for (col, cell) in row.iter().enumerate() {
            if col > 0 {
                line.push_str("  ");
            }
            line.push_str(cell);
            // Do not pad the last column.
            if col < 4 {
                line.push_str(&" ".repeat(widths[col] - cell.chars().count()));
            }
        }
        line
    });

    table.chain(notes).collect()
}

fn first_line(message: &str) -> &str {
    message.lines().next().unwrap_or(message)
}

fn changes(status: &RepoStatus) -> Vec<String> {
    let mut parts = Vec::new();
    if status.modified > 0 {
        parts.push(format!("{} modified", status.modified));
    }
    if status.staged > 0 {
        parts.push(format!("{} staged", status.staged));
    }
    if status.untracked > 0 {
        parts.push(format!("{} untracked", status.untracked));
    }
    parts
}

/// `clean`, or which kinds of change are present.
pub fn state_summary(status: &RepoStatus) -> String {
    if status.is_clean() {
        return "clean".into();
    }
    format!("dirty ({})", changes(status).join(", "))
}

fn sync_summary(status: &RepoStatus) -> String {
    if status.upstream.is_none() {
        return "-".into();
    }
    match (status.ahead, status.behind) {
        (0, 0) => "in sync".into(),
        (ahead, 0) => format!("ahead {ahead}"),
        (0, behind) => format!("behind {behind}"),
        (ahead, behind) => format!("ahead {ahead}, behind {behind}"),
    }
}

fn upstream_summary(status: &RepoStatus) -> String {
    match (&status.upstream, status.unpushed) {
        (Some(upstream), _) => upstream.clone(),
        (None, 0) => "none".into(),
        (None, n) => format!("none ({n} unpushed)"),
    }
}

/// Everything at risk in one repo, for the confirmation listing.
pub fn describe_risk(status: &RepoStatus) -> String {
    let mut parts = changes(status);
    if status.unpushed > 0 {
        let noun = if status.unpushed == 1 {
            "commit"
        } else {
            "commits"
        };
        let place = if status.upstream.is_some() {
            "not pushed"
        } else {
            "on no remote"
        };
        parts.push(format!("{} {noun} {place}", status.unpushed));
    }
    if parts.is_empty() {
        return "nothing".into();
    }
    format!("{} ({})", status.head_label(), parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(branch: &str) -> RepoStatus {
        RepoStatus {
            branch: Some(branch.into()),
            detached: false,
            head: Some("0123456789abcdef".into()),
            modified: 0,
            staged: 0,
            untracked: 0,
            upstream: Some("origin/main".into()),
            ahead: 0,
            behind: 0,
            unpushed: 0,
        }
    }

    #[test]
    fn table_renders_every_state() {
        let mut dirty = status("feature/PROJ-1");
        dirty.modified = 2;
        dirty.staged = 1;
        dirty.untracked = 3;
        dirty.ahead = 2;
        dirty.behind = 1;
        dirty.unpushed = 2;

        let mut no_upstream = status("scratch");
        no_upstream.upstream = None;
        no_upstream.unpushed = 4;

        let mut detached = status("x");
        detached.branch = None;
        detached.detached = true;
        detached.upstream = None;

        let rows = vec![
            StatusRow {
                project: "api".into(),
                status: Ok(status("main")),
            },
            StatusRow {
                project: "web".into(),
                status: Ok(dirty),
            },
            StatusRow {
                project: "tools".into(),
                status: Ok(no_upstream),
            },
            StatusRow {
                project: "old".into(),
                status: Ok(detached),
            },
            StatusRow {
                project: "broken".into(),
                status: Err("Failed to open repository at /x\nCaused by: nope".into()),
            },
        ];
        let lines = render_status_table(&rows);

        // Header, five rows, and one note for the unreadable project.
        assert_eq!(lines.len(), 7);
        assert!(lines[0].starts_with("PROJECT"));
        assert!(lines[1].starts_with("api "));
        assert!(lines[1].contains("main"));
        assert!(lines[1].contains("clean"));
        assert!(lines[1].contains("in sync"));
        assert!(lines[1].trim_end().ends_with("origin/main"));
        assert!(lines[2].contains("dirty (2 modified, 1 staged, 3 untracked)"));
        assert!(lines[2].contains("ahead 2, behind 1"));
        assert!(lines[3].contains("none (4 unpushed)"));
        assert!(lines[4].contains("detached at 01234567"));
        assert!(lines[5].contains("unreadable"));
        assert_eq!(lines[6], "! broken: Failed to open repository at /x");

        // Columns line up: every table row's BRANCH column starts at the same offset.
        let offset = lines[0].find("BRANCH").unwrap();
        for line in &lines[1..6] {
            assert_eq!(line.chars().nth(offset - 1), Some(' '), "{line}");
        }
    }

    #[test]
    fn risk_description_lists_each_kind() {
        let mut s = status("main");
        assert_eq!(describe_risk(&s), "nothing");
        s.modified = 1;
        s.untracked = 2;
        s.unpushed = 1;
        assert_eq!(
            describe_risk(&s),
            "main (1 modified, 2 untracked, 1 commit not pushed)"
        );
        s.upstream = None;
        s.unpushed = 3;
        assert!(describe_risk(&s).ends_with("3 commits on no remote)"));
    }
}

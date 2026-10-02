//! `de workspace list`: every saved workspace, which one is active, and which have services
//! running.
//!
//! The running dot comes from a single `docker compose ls` for all workspaces. When docker
//! cannot be asked there are simply no dots — the list itself never fails because of it.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    types::Slug,
    utils::ui::UserInterface,
    workspace::{health, registry},
};

pub fn list() -> eyre::Result<()> {
    let ui = UserInterface::new();
    let saved = registry::list_workspaces()?;

    if saved.entries.is_empty() && saved.unreadable.is_empty() {
        ui.info_item("No workspaces yet. Run `de init` in a project folder to create one.")?;
        return Ok(());
    }

    let active = registry::selected_name()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let running = running_dots(&saved);

    if !saved.entries.is_empty() {
        ui.heading("Workspaces")?;
    }
    let width = saved
        .entries
        .iter()
        .map(|e| e.name.as_str().len())
        .max()
        .unwrap_or(0);

    for entry in &saved.entries {
        let dot = match running.get(&entry.name) {
            Some(true) => ui.theme.success("●"),
            Some(false) => ui.theme.dim("○"),
            // Docker could not be asked; silence is better than a wrong dot.
            None => " ".to_string(),
        };
        let projects = if entry.projects == 0 {
            "no projects".to_string()
        } else {
            format!(
                "{} project{}",
                entry.projects,
                if entry.projects == 1 { "" } else { "s" }
            )
        };
        // `Slug`'s Display does not honour a width, so the string is padded first.
        let name = ui.theme.dim(&format!("{:1$}", entry.name.as_str(), width));
        let mut line = format!(
            "{dot} {name}  {:20}  {}",
            projects,
            last_used_line(now, entry.last_used)
        );
        if active.as_ref() == Some(&entry.name) {
            line.push_str(&format!("  {}", ui.theme.accent("(active)")));
        }
        ui.writeln(&line)?;
    }

    if !saved.unreadable.is_empty() {
        ui.new_line()?;
        ui.heading("Not readable")?;
        for bad in &saved.unreadable {
            ui.warning_item(
                &format!("{} — {}", bad.name, bad.reason),
                Some(&bad.path.display().to_string()),
            )?;
        }
    }

    Ok(())
}

/// Which workspaces have at least one compose project running, from one `docker compose ls`.
/// A workspace file that cannot be read is simply not checked (it is reported separately).
fn running_dots(
    saved: &registry::WorkspaceList,
) -> std::collections::HashMap<Slug, bool> {
    let mut loaded = Vec::new();
    for entry in &saved.entries {
        if let Ok(workspace) = registry::load_workspace(&entry.name) {
            loaded.push(workspace);
        }
    }
    let refs: Vec<&_> = loaded.iter().collect();
    health::running_workspaces(&refs).unwrap_or_default()
}

/// `never opened`, `just now`, `12m ago`, `3h ago`, `6d ago`.
fn last_used_line(now: i64, last_used: Option<i64>) -> String {
    let Some(last_used) = last_used else {
        return "never opened".into();
    };
    let age = (now - last_used).max(0);
    match age {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{}m ago", age / 60),
        3_600..=86_399 => format!("{}h ago", age / 3_600),
        _ => format!("{}d ago", age / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_of_a_last_use_reads_the_way_a_list_needs() {
        let now = 1_000_000;
        assert_eq!(last_used_line(now, None), "never opened");
        assert_eq!(last_used_line(now, Some(now)), "just now");
        assert_eq!(last_used_line(now, Some(now - 30)), "just now");
        assert_eq!(last_used_line(now, Some(now - 12 * 60)), "12m ago");
        assert_eq!(last_used_line(now, Some(now - 3 * 3_600)), "3h ago");
        assert_eq!(last_used_line(now, Some(now - 6 * 86_400)), "6d ago");
        // A clock that went backwards reads as "just now", never as a negative age.
        assert_eq!(last_used_line(now, Some(now + 500)), "just now");
    }
}

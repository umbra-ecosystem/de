//! Applying and reverting the composer test overlay.
//!
//! What it does to a consumer repo: `composer.json` gets a path repository (symlink) to the
//! provider's checkout and a `*` constraint for the package, then `composer update <package>`
//! rewrites `composer.lock` and `vendor/`. The exact original bytes of both files are stored
//! in `state.db` *before* anything changes, and revert works from that row alone.

use std::path::{Path, PathBuf};

use eyre::{Context, eyre};

use super::{
    OverlayError,
    composer::apply_overlay,
    runner::{CommandRunner, ExternalCommand, run_checked},
};
use crate::{
    domain::TicketKey,
    store::{Store, overlays, overlays::OverlayBackup},
};

/// A package the consumer gets from another workspace repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayPackage {
    /// Composer package name, e.g. `acme/api-client`.
    pub package: String,
    /// Name of the workspace project providing it.
    pub provider: String,
    /// The provider's checkout, which must have the ticket branch checked out.
    pub provider_dir: PathBuf,
}

/// Everything needed to apply the overlay in one consumer repo.
#[derive(Debug, Clone)]
pub struct ApplyRequest<'a> {
    pub ticket: &'a TicketKey,
    /// Workspace name of the consumer (the key of its backup).
    pub repo: &'a str,
    pub consumer_dir: &'a Path,
    pub packages: &'a [OverlayPackage],
    /// Rebuild steps, already resolved to commands, run once the overlay is in place.
    pub rebuild: &'a [ExternalCommand],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// The `url` each package's path repository received, in package order.
    pub urls: Vec<String>,
    /// Labels of the rebuild steps that ran.
    pub rebuilt: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevertOutcome {
    /// No overlay is recorded for this ticket and repo; nothing was touched.
    NothingToRevert,
    Reverted {
        /// `composer.lock` did not exist before the overlay and was deleted again.
        lock_removed: bool,
        /// Composer files that were edited while the overlay was applied. Their contents were
        /// saved under these names (next to the originals) before being replaced.
        saved: Vec<String>,
    },
}

/// `../name` when `provider` is a sibling directory of `consumer`, else its absolute path.
pub fn provider_url(consumer: &Path, provider: &Path) -> String {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
    let (consumer, provider) = (canon(consumer), canon(provider));

    match (consumer.parent(), provider.parent(), provider.file_name()) {
        (Some(a), Some(b), Some(name)) if a == b => format!("../{}", name.to_string_lossy()),
        _ => provider.to_string_lossy().into_owned(),
    }
}

fn read_optional(path: &Path) -> eyre::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(eyre::Report::new(e).wrap_err(format!("Failed to read {}", path.display()))),
    }
}

/// `<name>.de-edited`, or the first of `<name>.de-edited-2`, `-3`, ... that is free.
fn free_name(dir: &Path, name: &str) -> String {
    let mut candidate = format!("{name}.de-edited");
    let mut n = 2;
    while dir.join(&candidate).exists() {
        candidate = format!("{name}.de-edited-{n}");
        n += 1;
    }
    candidate
}

fn remove_if_exists(path: &Path) -> eyre::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => {
            Err(eyre::Report::new(e).wrap_err(format!("Failed to delete {}", path.display())))
        }
    }
}

/// Applies the overlay in `req.consumer_dir`.
///
/// Order: check nothing is applied yet, compute the new `composer.json` (any problem with it
/// surfaces before anything is written), back up the original bytes, write, run
/// `composer update`, then the rebuild steps.
///
/// If a step after the backup fails, the overlay is left applied *and recorded*, so
/// [`revert`] can undo it; callers that roll back do exactly that.
///
/// Applying twice for the same ticket and repo, or on a checkout that already carries an
/// overlay, is refused with [`OverlayError::AlreadyApplied`]: a second backup would capture
/// the overlaid files and make the overlay permanent.
pub fn apply(
    store: &Store,
    runner: &dyn CommandRunner,
    req: &ApplyRequest<'_>,
    now: i64,
) -> eyre::Result<ApplyOutcome> {
    let dir = req.consumer_dir;

    if let Some(existing) = overlays::find_by_dir(store, dir)? {
        return Err(OverlayError::AlreadyApplied {
            by: existing.ticket,
            repo: existing.repo,
        }
        .into());
    }
    if let Some(existing) = overlays::get(store, req.ticket, req.repo)? {
        return Err(OverlayError::AlreadyApplied {
            by: existing.ticket,
            repo: existing.repo,
        }
        .into());
    }

    let json_path = dir.join("composer.json");
    let lock_path = dir.join("composer.lock");
    let original_json = read_optional(&json_path)?
        .ok_or_else(|| OverlayError::MissingComposerJson(json_path.clone()))?;
    let original_lock = read_optional(&lock_path)?;

    let urls: Vec<String> = req
        .packages
        .iter()
        .map(|p| provider_url(dir, &p.provider_dir))
        .collect();
    let entries: Vec<(String, String)> = req
        .packages
        .iter()
        .zip(&urls)
        .map(|(p, url)| (p.package.clone(), url.clone()))
        .collect();
    let new_json = apply_overlay(&original_json, &entries)?;

    let description: Vec<serde_json::Value> = req
        .packages
        .iter()
        .zip(&urls)
        .map(|(p, url)| {
            serde_json::json!({
                "package": p.package,
                "provider": p.provider,
                "provider_dir": p.provider_dir,
                "url": url,
            })
        })
        .collect();
    overlays::insert(
        store,
        &OverlayBackup {
            ticket: req.ticket.clone(),
            repo: req.repo.into(),
            repo_dir: dir.into(),
            packages: serde_json::Value::Array(description).to_string(),
            composer_json: original_json,
            composer_lock: original_lock,
            json_after: None,
            lock_after: None,
            files_restored: false,
            created_at: now,
        },
    )?;

    std::fs::write(&json_path, &new_json)
        .wrap_err_with(|| format!("Failed to write {}", json_path.display()))?;
    // Known from here on, so a revert can tell the overlay's file from one edited afterwards.
    overlays::set_after(store, req.ticket, req.repo, &new_json, None)?;

    let names: Vec<&str> = req.packages.iter().map(|p| p.package.as_str()).collect();
    let mut update_args = vec!["update"];
    update_args.extend(names);
    run_checked(runner, &ExternalCommand::composer(dir, &update_args))?;

    overlays::set_after(
        store,
        req.ticket,
        req.repo,
        &new_json,
        read_optional(&lock_path)?.as_deref(),
    )?;

    let mut rebuilt = Vec::new();
    for step in req.rebuild {
        run_checked(runner, step)?;
        rebuilt.push(step.label.clone());
    }

    Ok(ApplyOutcome { urls, rebuilt })
}

/// Puts the consumer back the way it was before [`apply`], from the stored backup alone (so
/// it works from a fresh process after a crash): restores `composer.json` and `composer.lock`
/// byte for byte (deleting the lock if there was none), runs `composer install` so `vendor/`
/// no longer points at the symlink, then forgets the backup.
///
/// Idempotent. Without a backup it does nothing and says so. If `composer install` fails the
/// files are already restored and the backup is kept, so a retry only repeats the install.
pub fn revert(
    store: &Store,
    runner: &dyn CommandRunner,
    ticket: &TicketKey,
    repo: &str,
) -> eyre::Result<RevertOutcome> {
    let Some(backup) = overlays::get(store, ticket, repo)? else {
        return Ok(RevertOutcome::NothingToRevert);
    };
    let dir = &backup.repo_dir;
    let json_path = dir.join("composer.json");
    let lock_path = dir.join("composer.lock");

    let mut saved = Vec::new();
    if !backup.files_restored {
        // Anything that is neither the original nor what the overlay wrote is the user's own
        // edit made while testing; keep a copy before the original goes back.
        let edits = [
            (
                &json_path,
                "composer.json",
                backup.composer_json.as_slice(),
                backup.json_after.as_deref(),
            ),
            (
                &lock_path,
                "composer.lock",
                backup.composer_lock.as_deref().unwrap_or_default(),
                backup.lock_after.as_deref(),
            ),
        ];
        for (path, name, original, after) in edits {
            let Some(current) = read_optional(path)? else {
                continue;
            };
            let was_absent = name == "composer.lock" && backup.composer_lock.is_none();
            let is_original = current.as_slice() == original && !was_absent;
            if is_original || after == Some(current.as_slice()) {
                continue;
            }
            let copy = free_name(dir, name);
            std::fs::write(dir.join(&copy), &current)
                .wrap_err_with(|| format!("Failed to save the edited {}", path.display()))?;
            saved.push(copy);
        }

        std::fs::write(&json_path, &backup.composer_json)
            .wrap_err_with(|| format!("Failed to restore {}", json_path.display()))?;
        match &backup.composer_lock {
            Some(lock) => std::fs::write(&lock_path, lock)
                .wrap_err_with(|| format!("Failed to restore {}", lock_path.display()))?,
            None => {
                remove_if_exists(&lock_path)?;
            }
        }
        overlays::mark_files_restored(store, ticket, repo)?;
    }

    run_checked(runner, &ExternalCommand::composer(dir, &["install"])).wrap_err_with(|| {
        eyre!(
            "The composer files of {repo} are restored, but vendor/ could not be rebuilt; \
             fix the problem and revert again"
        )
    })?;

    // Without a lock to begin with, `composer install` just wrote one; it is not the original.
    let lock_removed = backup.composer_lock.is_none() && remove_if_exists(&lock_path)?;

    overlays::delete(store, ticket, repo)?;
    Ok(RevertOutcome::Reverted {
        lock_removed,
        saved,
    })
}

use crate::utils::get_shims_dir;
use eyre::{WrapErr, eyre};

pub fn list() -> eyre::Result<()> {
    let shims_dir = get_shims_dir()?;

    if !shims_dir.exists() {
        println!("No shims found.");
        return Ok(());
    }

    let entries = std::fs::read_dir(&shims_dir)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to read shims directory: {}", shims_dir.display()))?;

    let mut shims = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| eyre!(e)).wrap_err_with(|| {
            format!(
                "Failed to read entry in shims directory: {}",
                shims_dir.display()
            )
        })?;
        if let Some(name) = entry.file_name().to_str() {
            shims.push(name.to_string());
        }
    }

    if shims.is_empty() {
        println!("No shims found.");
        return Ok(());
    }

    println!("Shims in directory '{}':", shims_dir.display());
    for shim in shims {
        println!(" - {}", shim);
    }

    Ok(())
}

use crate::{types::Slug, utils::get_shims_dir};
use eyre::{WrapErr, eyre};

pub fn remove(command: Slug) -> eyre::Result<()> {
    let shims_dir = get_shims_dir()?;
    let shim_path = shims_dir.join(format!("{command}"));

    if shim_path.exists() {
        std::fs::remove_file(&shim_path)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| format!("Failed to remove shim at {}", shim_path.display()))?;
        println!("Removed shim: {command}");
    } else {
        println!("No shim found for command: {command}");
    }

    Ok(())
}

use std::str::FromStr;

use eyre::{WrapErr, eyre};

use crate::{
    types::Slug,
    utils::{
        formatter::Formatter,
        shim::{get_installed_shims, write_shim_to_file},
    },
};

pub fn reinstate() -> eyre::Result<()> {
    let formatter = Formatter::new();

    let installed_ships = get_installed_shims()?;
    if installed_ships.is_empty() {
        println!("No shims found to reinstate.");
        return Ok(());
    }

    formatter.heading("Reinstating shims:")?;
    for shim in installed_ships {
        let result = reinstate_shim(&shim);
        match result {
            Ok(()) => {
                formatter.success(&shim)?;
            }
            Err(e) => {
                formatter.error(
                    &format!("Failed to reinstate shim for command '{shim}'"),
                    Some(&e.to_string()),
                )?;
            }
        }
    }

    Ok(())
}

fn reinstate_shim(file_name: &str) -> eyre::Result<()> {
    let command = Slug::from_str(file_name)
        .map_err(|e| eyre!(e))
        .wrap_err("Invalid command name")?;

    write_shim_to_file(&command)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| format!("Failed to write shim for command: {command}"))?;

    Ok(())
}

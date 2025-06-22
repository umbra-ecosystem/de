use eyre::{Context, eyre};

use crate::project::Project;

pub fn run(task_name: String) -> eyre::Result<()> {
    let project = Project::current()
        .map_err(|e| eyre!(e))
        .wrap_err("Failed to get current project")?
        .ok_or_else(|| eyre!("No current project found"))?;
    
    

    Ok(())
}

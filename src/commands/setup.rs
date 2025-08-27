use crate::project::Project;

pub fn check() -> eyre::Result<()> {
    let config = Project::current()?
        .ok_or_else(|| eyre::eyre!("No project found in the current directory"))?;

    let manifest = config.manifest();
    println!("{:?}", config.setup());

    Ok(())
}

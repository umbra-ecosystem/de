use crate::project::Project;

pub fn check() -> eyre::Result<()> {
    let config = Project::current()
        .expect("Failed to load current project")
        .expect("No current project found");

    let manifest = config.manifest();
    println!("{:?}", config.setup());

    Ok(())
}

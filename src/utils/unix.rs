use eyre::eyre;

pub fn get_shell_config_paths() -> eyre::Result<Vec<std::path::PathBuf>> {
    let user_dirs =
        directories::UserDirs::new().ok_or_else(|| eyre!("Failed to get user directories"))?;
    let home_dir = user_dirs.home_dir();

    if cfg!(target_os = "linux") {
        Ok(vec![home_dir.join(".bashrc"), home_dir.join(".zshrc")])
    } else if cfg!(target_os = "macos") {
        Ok(vec![
            home_dir.join(".zshrc"),
            home_dir.join(".bash_profile"),
        ])
    } else {
        Err(eyre!(
            "Unsupported operating system for shell configuration"
        ))
    }
}

pub fn primary_shell_config_path() -> eyre::Result<std::path::PathBuf> {
    let user_dirs =
        directories::UserDirs::new().ok_or_else(|| eyre!("Failed to get user directories"))?;
    let home_dir = user_dirs.home_dir();

    if cfg!(target_os = "linux") {
        Ok(home_dir.join(".bashrc"))
    } else if cfg!(target_os = "macos") {
        Ok(home_dir.join(".zshrc"))
    } else {
        Err(eyre!(
            "Unsupported operating system for primary shell configuration"
        ))
    }
}

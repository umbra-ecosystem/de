use std::process::Command;

use eyre::eyre;

pub fn run_git_command(args: &[&str], dir: &std::path::Path) -> eyre::Result<()> {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    for arg in args {
        command.arg(arg);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(eyre!(
            "Git command failed: {}\n{}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

pub fn branch_exists(branch: &str, dir: &std::path::Path) -> eyre::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("branch")
        .arg("--list")
        .arg(branch)
        .output()?;
    let remote_output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("branch")
        .arg("-r")
        .arg("--list")
        .arg(format!("origin/{}", branch))
        .output()?;
    Ok(!output.stdout.is_empty() || !remote_output.stdout.is_empty())
}

pub fn get_default_branch(dir: &std::path::Path) -> eyre::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("origin/HEAD")
        .output()?;
    if !output.status.success() {
        return Err(eyre!("Failed to get default branch"));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string()
        .replace("origin/", ""))
}

pub fn get_current_branch(dir: &std::path::Path) -> eyre::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()?;
    if !output.status.success() {
        return Err(eyre::eyre!("Failed to get current branch"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn is_project_dirty(dir: &std::path::Path) -> eyre::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("status")
        .arg("--porcelain")
        .output()?;
    Ok(!output.stdout.is_empty())
}

pub fn has_unpushed_commits(branch: &str, dir: &std::path::Path) -> eyre::Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-list")
        .arg("--count")
        .arg(&format!("origin/{}..{}", branch, branch))
        .output()?;
    if !output.status.success() {
        return Err(eyre::eyre!("Failed to check for unpushed commits"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() != "0")
}

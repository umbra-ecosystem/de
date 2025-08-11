use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

use eyre::{WrapErr, eyre};
use serde::{Deserialize, Serialize};

use super::{project::CommandPipe, utils::EnvMapper};

#[derive(Debug, Clone, Deserialize, Serialize)]

pub struct ExportCommand {
    pub command: String,
    #[serde(default)]
    pub stdout: Option<CommandPipe>,
}

impl From<String> for ExportCommand {
    fn from(command: String) -> Self {
        Self {
            command,
            stdout: None,
        }
    }
}

pub enum ExportCommandResult {
    File { file_path: PathBuf },
    NoOutput,
}

impl ExportCommand {
    pub fn run(
        &self,
        dir: &Path,
        env_mapper: &EnvMapper,
        output_dir: &Path,
    ) -> eyre::Result<ExportCommandResult> {
        let command_str = env_mapper.format_str(&self.command);
        let mut parts = command_str.split_whitespace();
        let program = parts
            .next()
            .ok_or_else(|| eyre!("Command is empty or does not contain a program to run"))?;

        let mut command = std::process::Command::new(&program);
        command.current_dir(dir);
        command.args(parts);

        if let Some(stdout) = &self.stdout {
            match stdout {
                CommandPipe::File { file: file_name } => {
                    let (file_path, file) = resolve_pipe_file(file_name, env_mapper, output_dir)?;
                    command.stdout(file);

                    let status = command.status().map_err(|e| eyre!(e)).wrap_err_with(|| {
                        format!(
                            "Failed to run command: {} with output file: {}",
                            command_str, file_name
                        )
                    })?;

                    if !status.success() {
                        return Err(eyre!("Command failed with status: {}", status));
                    }

                    Ok(ExportCommandResult::File { file_path })
                }
            }
        } else {
            let status = command
                .status()
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to run command: {}", command_str))?;

            if !status.success() {
                return Err(eyre!("Command failed with status: {}", status));
            }

            Ok(ExportCommandResult::NoOutput)
        }
    }
}

fn resolve_pipe_file(
    file_name: &str,
    env_mapper: &EnvMapper,
    output_dir: &Path,
) -> eyre::Result<(PathBuf, File)> {
    let file_name = env_mapper.format_str(file_name);
    let file_path = output_dir.join(file_name);

    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)
            .map_err(|e| eyre!(e))
            .wrap_err_with(|| {
                format!("Failed to create output directory {}", output_dir.display())
            })?;
    }

    let file = File::create(&file_path)
        .map_err(|e| eyre!(e))
        .wrap_err_with(|| {
            format!(
                "Failed to create file for command output: {}",
                file_path.display()
            )
        })?;

    Ok((file_path, file))
}

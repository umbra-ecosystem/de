use std::{
    fs::File,
    path::{Path, PathBuf},
};

use eyre::{WrapErr, eyre};
use serde::{Deserialize, Serialize};

use crate::utils::path::has_reverse_path_traversal;

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
    pub fn resolve_env(&self, env_mapper: &EnvMapper) -> Self {
        Self {
            command: env_mapper.format_str(&self.command),
            stdout: self.stdout.as_ref().map(|pipe| match pipe {
                CommandPipe::File { file } => CommandPipe::File {
                    file: env_mapper.format_str(file),
                },
            }),
        }
    }

    pub fn run(
        &self,
        dir: &Path,
        output_dir: &Path,
        project_dir: &Path,
    ) -> eyre::Result<ExportCommandResult> {
        // TODO: add docker service support

        tracing::info!(
            "Running ExportCommand: '{}' in directory '{}'",
            self.command,
            dir.display()
        );

        let mut parts = self.command.split_whitespace();
        let program = parts
            .next()
            .ok_or_else(|| eyre!("Command is empty or does not contain a program to run"))?;

        let mut command = std::process::Command::new(program);
        command.current_dir(dir);
        command.args(parts);

        if let Some(stdout) = &self.stdout {
            match stdout {
                CommandPipe::File { file: file_name } => {
                    let (file_path, file) = resolve_pipe_file(file_name, output_dir)?;
                    command.stdout(file);

                    let status = command.status().map_err(|e| eyre!(e)).wrap_err_with(|| {
                        format!(
                            "Failed to run command: {} with output file: {}",
                            self.command, file_name
                        )
                    })?;

                    if !status.success() {
                        tracing::info!(
                            "ExportCommand failed: '{}' (status: {})",
                            self.command,
                            status
                        );

                        return Err(eyre!("Command failed with status: {}", status));
                    }

                    let file_path = file_path
                        .strip_prefix(project_dir)
                        .map_err(|e| eyre!(e))
                        .wrap_err_with(|| {
                            format!(
                                "Failed to strip prefix from file path: {}",
                                file_path.display()
                            )
                        })?;

                    tracing::info!(
                        "ExportCommand succeeded: '{}' (output path: '{}')",
                        self.command,
                        file_path.display()
                    );

                    Ok(ExportCommandResult::File {
                        file_path: file_path.to_path_buf(),
                    })
                }
            }
        } else {
            let status = command
                .status()
                .map_err(|e| eyre!(e))
                .wrap_err_with(|| format!("Failed to run command: {}", self.command))?;

            if !status.success() {
                tracing::info!(
                    "ExportCommand failed: '{}' (status: {})",
                    self.command,
                    status
                );
                return Err(eyre!("Command failed with status: {}", status));
            } else {
                tracing::info!(
                    "ExportCommand succeeded: '{}' (no output file)",
                    self.command
                );
            }

            Ok(ExportCommandResult::NoOutput)
        }
    }
}

fn resolve_pipe_file(file_name: &str, output_dir: &Path) -> eyre::Result<(PathBuf, File)> {
    let file_path = output_dir.join(file_name);

    // SECURITY: Ensure the file path is within the output_dir
    if has_reverse_path_traversal(&file_path) {
        return Err(eyre!(
            "Invalid file path '{}': contains reverse path traversal",
            file_path.display()
        ));
    }

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

    tracing::info!(
        "Resolved pipe file for ExportCommand output: '{}'",
        file_path.display()
    );

    Ok((file_path, file))
}

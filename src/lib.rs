pub mod atomic;
pub mod cli;
pub mod commands;
pub mod config;
pub mod control_store;
pub mod discovery;
pub mod domain;
pub mod drift;
pub mod error;
pub mod fs_plan;
pub mod git_repo;
pub mod lock;
pub mod manifest;
pub mod materialize;
pub mod mode_plan;
pub mod state;
pub mod sync_plan;
pub mod ui;

use std::process::ExitCode;

use camino::Utf8PathBuf;
use clap::Parser as _;

use crate::{cli::Cli, commands::run_command, error::WkError, ui::CliclackPrompter};

pub fn run() -> Result<ExitCode, WkError> {
    let cli = Cli::parse();
    let cwd = Utf8PathBuf::from_path_buf(std::env::current_dir()?)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))?;
    let prompter = CliclackPrompter::new(cli.non_interactive);
    run_command(cli, &cwd, &prompter)
}

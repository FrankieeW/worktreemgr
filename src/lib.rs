pub mod cli;
pub mod config;
pub mod control_store;
pub mod discovery;
pub mod domain;
pub mod error;
pub mod fs_plan;
pub mod git_repo;
pub mod manifest;

use clap::Parser as _;

use crate::{
    cli::{Cli, Command},
    error::WkError,
};

pub fn run() -> Result<(), WkError> {
    let cli = Cli::parse();
    run_cli(&cli)
}

pub const fn run_cli(cli: &Cli) -> Result<(), WkError> {
    match &cli.command {
        Command::Init
        | Command::Add { .. }
        | Command::Apply { .. }
        | Command::Status { .. }
        | Command::Sync { .. }
        | Command::Mode { .. }
        | Command::Prune
        | Command::Gc { .. } => Ok(()),
    }
}

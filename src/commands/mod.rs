pub mod add;
pub mod apply;
pub mod gc;
pub mod init;
pub mod mode;
pub mod prune;
pub mod status;
pub mod sync;

use std::process::ExitCode;

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    atomic::ensure_private_dir,
    cli::{Cli, Command},
    config::{Config, PathConfig, load_config, save_config_atomic},
    domain::{ManagedPath, Mode, SyncPolicy},
    error::WkError,
    fs_plan::execute_plan,
    git_repo::{RepoContext, discover_repo},
    lock::MutationLock,
    materialize::{OperationPlan, StateUpdate},
    state::StateStore,
    ui::Prompter,
};

pub fn run_command(cli: Cli, cwd: &Utf8Path, prompter: &dyn Prompter) -> Result<ExitCode, WkError> {
    let ctx = discover_repo(cwd)?;
    let dry_run = cli.dry_run;
    let command = cli.command;
    let _lock = if command_mutates(&command, dry_run) {
        Some(MutationLock::acquire(&ctx.control_dir)?)
    } else {
        None
    };
    match command {
        Command::Init => init::run(&ctx, prompter),
        Command::Add { path, force } => add::run(&ctx, prompter, &path, force),
        Command::Apply { worktree } => apply::run(&ctx, worktree.as_deref(), dry_run),
        Command::Status { json } => status::run(&ctx, json),
        Command::Sync { path, worktree } => {
            sync::run(&ctx, path.as_deref(), worktree.as_deref(), dry_run)
        }
        Command::Mode {
            path,
            mode,
            sync_policy,
            conflict_policy,
            choice,
        } => mode::run(
            &ctx,
            &path,
            mode,
            mode::ModeCommandOptions {
                sync_policy,
                conflict_policy,
                choice,
                dry_run,
            },
        ),
        Command::Prune => prune::run(&ctx),
        Command::Gc {
            force,
            older_than,
            keep,
        } => gc::run(&ctx, force, older_than.as_deref(), keep, dry_run),
    }
}

const fn command_mutates(command: &Command, dry_run: bool) -> bool {
    match command {
        Command::Init | Command::Add { .. } | Command::Prune => true,
        Command::Apply { .. } | Command::Sync { .. } | Command::Mode { .. } => !dry_run,
        Command::Gc { force, .. } => *force && !dry_run,
        Command::Status { .. } => false,
    }
}

pub(crate) fn config_path(ctx: &RepoContext) -> Utf8PathBuf {
    ctx.control_dir.join("config.toml")
}

pub(crate) const fn default_config() -> Config {
    Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: crate::domain::ConflictPolicy::Ask,
        paths: Vec::new(),
    }
}

pub(crate) fn load_config_or_default(ctx: &RepoContext) -> Result<Config, WkError> {
    let path = config_path(ctx);
    if path.exists() {
        return load_config(&path);
    }
    Ok(default_config())
}

pub(crate) fn save_config(ctx: &RepoContext, config: &Config) -> Result<(), WkError> {
    ensure_private_dir(&ctx.control_dir)?;
    save_config_atomic(&config_path(ctx), config)
}

pub(crate) fn prompt_path_config(
    path: ManagedPath,
    prompter: &dyn Prompter,
) -> Result<PathConfig, WkError> {
    let mode = prompter.select_mode(&path)?;
    let (sync_policy, conflict_policy) = if mode == Mode::Sync {
        (
            Some(prompter.select_sync_policy(&path)?),
            Some(prompter.select_conflict_policy(&path)?),
        )
    } else {
        (None, None)
    };
    Ok(PathConfig {
        path,
        mode,
        sync_policy,
        conflict_policy,
    })
}

pub(crate) fn persist_state_updates(
    state: &StateStore,
    updates: &[StateUpdate],
) -> Result<(), WkError> {
    for update in updates {
        match update {
            StateUpdate::Save(path_state) => state.save_path_state(path_state)?,
            StateUpdate::Remove { path, worktree_id } => {
                state.remove_path_state(path, worktree_id)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn execute_and_persist(
    plan: &OperationPlan,
    state: &StateStore,
    dry_run: bool,
) -> Result<(), WkError> {
    let report = execute_plan(&plan.fs_ops, dry_run)?;
    for summary in &plan.summary {
        println!("{summary}");
    }
    for operation in report.operations {
        println!("{operation}");
    }
    for warning in plan.warnings.iter().chain(report.warnings.iter()) {
        eprintln!("warning: {warning}");
    }
    if !dry_run {
        persist_state_updates(state, &plan.state_updates)?;
    }
    Ok(())
}

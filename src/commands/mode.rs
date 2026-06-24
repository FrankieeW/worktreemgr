use std::process::ExitCode;

use crate::{
    cli::{CliConflictPolicy, CliMode, CliSyncPolicy, CliTransitionChoice},
    commands::{execute_and_persist, load_config_or_default, save_config},
    config::{Config, PathConfig},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    error::WkError,
    git_repo::RepoContext,
    materialize::OperationPlan,
    mode_plan::{ModeOptions, TransitionChoice, plan_mode_change},
    state::StateStore,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeCommandOptions {
    pub sync_policy: Option<CliSyncPolicy>,
    pub conflict_policy: Option<CliConflictPolicy>,
    pub choice: Option<CliTransitionChoice>,
    pub dry_run: bool,
}

pub fn run(
    ctx: &RepoContext,
    raw_path: &str,
    target_mode: CliMode,
    options: ModeCommandOptions,
) -> Result<ExitCode, WkError> {
    let path = ManagedPath::parse(raw_path)?;
    let target_mode = cli_mode(target_mode);
    let mut config = load_config_or_default(ctx)?;
    let conflict_policy = options.conflict_policy.map(cli_conflict_policy);
    if conflict_policy == Some(ConflictPolicy::Newer) {
        eprintln!("warning: newer conflict policy uses unsafe mtime comparison");
    }
    let state = StateStore::new(&ctx.control_dir);
    let plan = plan_mode(
        ctx,
        &config,
        &state,
        &path,
        target_mode,
        ModeOptions {
            dry_run: options.dry_run,
            choice: options
                .choice
                .map_or(TransitionChoice::Default, cli_transition_choice),
        },
    )?;
    execute_and_persist(&plan, &state, options.dry_run)?;
    if !options.dry_run && should_update_config(&config, &path, target_mode, &plan) {
        upsert_config(
            &mut config,
            path,
            target_mode,
            options.sync_policy.map(cli_sync_policy),
            conflict_policy,
        );
        save_config(ctx, &config)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn should_update_config(
    config: &Config,
    path: &ManagedPath,
    target_mode: Mode,
    plan: &OperationPlan,
) -> bool {
    let current_mode = config
        .paths
        .iter()
        .find(|path_config| path_config.path == *path)
        .map_or(Mode::Ignore, |path_config| path_config.mode);
    if current_mode == Mode::Sync
        && target_mode == Mode::Link
        && plan.fs_ops.is_empty()
        && plan.state_updates.is_empty()
    {
        return false;
    }
    true
}

fn plan_mode(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    path: &ManagedPath,
    target_mode: Mode,
    options: ModeOptions,
) -> Result<OperationPlan, WkError> {
    plan_mode_change(ctx, config, state, path, target_mode, options)
}

fn upsert_config(
    config: &mut Config,
    path: ManagedPath,
    mode: Mode,
    sync_policy: Option<SyncPolicy>,
    conflict_policy: Option<ConflictPolicy>,
) {
    if let Some(existing) = config
        .paths
        .iter_mut()
        .find(|path_config| path_config.path == path)
    {
        existing.mode = mode;
        existing.sync_policy = sync_policy;
        existing.conflict_policy = conflict_policy;
        return;
    }
    config.paths.push(PathConfig {
        path,
        mode,
        sync_policy,
        conflict_policy,
    });
}

const fn cli_mode(mode: CliMode) -> Mode {
    match mode {
        CliMode::Ignore => Mode::Ignore,
        CliMode::Link => Mode::Link,
        CliMode::Copy => Mode::Copy,
        CliMode::Sync => Mode::Sync,
    }
}

const fn cli_sync_policy(policy: CliSyncPolicy) -> SyncPolicy {
    match policy {
        CliSyncPolicy::Manual => SyncPolicy::Manual,
        CliSyncPolicy::Auto => SyncPolicy::Auto,
    }
}

const fn cli_conflict_policy(policy: CliConflictPolicy) -> ConflictPolicy {
    match policy {
        CliConflictPolicy::Ask => ConflictPolicy::Ask,
        CliConflictPolicy::Source => ConflictPolicy::Source,
        CliConflictPolicy::Worktree => ConflictPolicy::Worktree,
        CliConflictPolicy::Newer => ConflictPolicy::Newer,
    }
}

const fn cli_transition_choice(choice: CliTransitionChoice) -> TransitionChoice {
    match choice {
        CliTransitionChoice::Default => TransitionChoice::Default,
        CliTransitionChoice::Source => TransitionChoice::SourceWins,
        CliTransitionChoice::Worktree => TransitionChoice::WorktreeWins,
        CliTransitionChoice::Skip => TransitionChoice::Skip,
    }
}

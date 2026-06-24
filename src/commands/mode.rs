use std::process::ExitCode;

use crate::{
    cli::{CliConflictPolicy, CliMode, CliSyncPolicy},
    commands::{execute_and_persist, load_config_or_default, save_config},
    config::{Config, PathConfig},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    error::WkError,
    fs_plan::FsOp,
    git_repo::{RepoContext, WorktreeInfo},
    materialize::{OperationPlan, plan_overlay_to_worktree},
    mode_plan::{ModeOptions, TransitionChoice, plan_mode_change},
    state::StateStore,
};

pub fn run(
    ctx: &RepoContext,
    raw_path: &str,
    target_mode: CliMode,
    sync_policy: Option<CliSyncPolicy>,
    conflict_policy: Option<CliConflictPolicy>,
    dry_run: bool,
) -> Result<ExitCode, WkError> {
    let path = ManagedPath::parse(raw_path)?;
    let target_mode = cli_mode(target_mode);
    let mut config = load_config_or_default(ctx)?;
    let conflict_policy = conflict_policy.map(cli_conflict_policy);
    if conflict_policy == Some(ConflictPolicy::Newer) {
        eprintln!("warning: newer conflict policy uses unsafe mtime comparison");
    }
    let state = StateStore::new(&ctx.control_dir);
    let plan = plan_mode(ctx, &config, &state, &path, target_mode, dry_run)?;
    execute_and_persist(&plan, &state, dry_run)?;
    if !dry_run {
        upsert_config(
            &mut config,
            path,
            target_mode,
            sync_policy.map(cli_sync_policy),
            conflict_policy,
        );
        save_config(ctx, &config)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn plan_mode(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    path: &ManagedPath,
    target_mode: Mode,
    dry_run: bool,
) -> Result<OperationPlan, WkError> {
    let current_mode = config
        .paths
        .iter()
        .find(|path_config| path_config.path == *path)
        .map_or(Mode::Ignore, |path_config| path_config.mode);
    if current_mode == Mode::Link && target_mode == Mode::Copy {
        return plan_link_to_copy(ctx, path, dry_run);
    }
    plan_mode_change(
        ctx,
        config,
        state,
        path,
        target_mode,
        ModeOptions {
            dry_run,
            choice: TransitionChoice::Default,
        },
    )
}

fn plan_link_to_copy(
    ctx: &RepoContext,
    path: &ManagedPath,
    dry_run: bool,
) -> Result<OperationPlan, WkError> {
    let mut plan = OperationPlan::default();
    for worktree in ctx.non_source_worktrees() {
        remove_link_if_present(&mut plan, path, worktree)?;
        plan.extend(plan_overlay_to_worktree(ctx, path, worktree)?);
    }
    if dry_run {
        plan.summary
            .push("dry run: no filesystem writes".to_owned());
    }
    Ok(plan)
}

fn remove_link_if_present(
    plan: &mut OperationPlan,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<(), WkError> {
    let dest = worktree.path.join(path.as_path());
    match std::fs::symlink_metadata(&dest) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            plan.fs_ops.push(FsOp::RemoveFile { path: dest });
        }
        Ok(_metadata) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
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

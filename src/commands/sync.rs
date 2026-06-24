use std::process::ExitCode;

use crate::{
    commands::{execute_and_persist, load_config_or_default},
    config::{Config, PathConfig},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    error::WkError,
    git_repo::{RepoContext, WorktreeId},
    materialize::OperationPlan,
    state::StateStore,
    sync_plan::{SyncOptions, SyncSelector, plan_sync},
};

pub fn run(
    ctx: &RepoContext,
    path: Option<&str>,
    worktree: Option<&str>,
    dry_run: bool,
) -> Result<ExitCode, WkError> {
    let config = load_config_or_default(ctx)?;
    let state = StateStore::new(&ctx.control_dir);
    let plan = match selector(ctx, &config, path, worktree)? {
        Selector::All => plan_for_all(ctx, &config, &state, None, dry_run)?,
        Selector::Worktree(worktree_id) => {
            plan_for_all(ctx, &config, &state, Some(&worktree_id), dry_run)?
        }
        Selector::Path(path) => plan_for_path(ctx, &config, &state, &path, None, dry_run)?,
        Selector::PathAndWorktree(path, worktree_id) => {
            plan_for_path(ctx, &config, &state, &path, Some(&worktree_id), dry_run)?
        }
    };
    execute_and_persist(&plan, &state, dry_run)?;
    Ok(ExitCode::SUCCESS)
}

enum Selector {
    All,
    Path(ManagedPath),
    Worktree(WorktreeId),
    PathAndWorktree(ManagedPath, WorktreeId),
}

fn selector(
    ctx: &RepoContext,
    config: &Config,
    path: Option<&str>,
    worktree: Option<&str>,
) -> Result<Selector, WkError> {
    match (path, worktree) {
        (None, None) => Ok(Selector::All),
        (Some(raw), None) => {
            let managed = ManagedPath::parse(raw)?;
            if has_path(config, &managed) {
                return Ok(Selector::Path(managed));
            }
            Ok(Selector::Worktree(resolve_worktree(ctx, raw)?))
        }
        (Some(raw_path), Some(raw_worktree)) => Ok(Selector::PathAndWorktree(
            ManagedPath::parse(raw_path)?,
            resolve_worktree(ctx, raw_worktree)?,
        )),
        (None, Some(raw_worktree)) => Ok(Selector::Worktree(resolve_worktree(ctx, raw_worktree)?)),
    }
}

fn plan_for_all(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    worktree: Option<&WorktreeId>,
    dry_run: bool,
) -> Result<OperationPlan, WkError> {
    let mut plan = OperationPlan::default();
    for path_config in config.paths.iter().filter(|item| item.mode == Mode::Sync) {
        plan.extend(plan_for_path(
            ctx,
            config,
            state,
            &path_config.path,
            worktree,
            dry_run,
        )?);
    }
    Ok(plan)
}

fn plan_for_path(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    path: &ManagedPath,
    worktree: Option<&WorktreeId>,
    dry_run: bool,
) -> Result<OperationPlan, WkError> {
    let path_config = config
        .paths
        .iter()
        .find(|item| item.path == *path)
        .ok_or_else(|| WkError::message(format!("path is not configured: {path}")))?;
    let selector = worktree.map_or_else(
        || SyncSelector::Path(path.clone()),
        |worktree_id| SyncSelector::PathAndWorktree(path.clone(), worktree_id.clone()),
    );
    plan_sync(
        ctx,
        config,
        state,
        &selector,
        SyncOptions {
            policy: effective_sync_policy(config, path_config),
            conflict_policy: effective_conflict_policy(config, path_config),
            dry_run,
        },
    )
}

fn has_path(config: &Config, path: &ManagedPath) -> bool {
    config.paths.iter().any(|item| item.path == *path)
}

fn resolve_worktree(ctx: &RepoContext, raw: &str) -> Result<WorktreeId, WkError> {
    ctx.non_source_worktrees()
        .find(|worktree| worktree.id.as_str() == raw || worktree.path.as_str() == raw)
        .map(|worktree| worktree.id.clone())
        .ok_or_else(|| WkError::message(format!("unknown worktree: {raw}")))
}

fn effective_sync_policy(config: &Config, path_config: &PathConfig) -> SyncPolicy {
    path_config
        .sync_policy
        .unwrap_or(config.default_sync_policy)
}

fn effective_conflict_policy(config: &Config, path_config: &PathConfig) -> ConflictPolicy {
    path_config
        .conflict_policy
        .unwrap_or(config.default_conflict_policy)
}

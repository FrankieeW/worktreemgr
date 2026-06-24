use std::process::ExitCode;

use crate::{
    commands::{execute_and_persist, load_config_or_default},
    config::{Config, PathConfig},
    domain::{ConflictPolicy, Mode, SyncPolicy},
    error::WkError,
    git_repo::RepoContext,
    materialize::{ApplyTarget, OperationPlan, plan_apply},
    state::StateStore,
    sync_plan::{SyncOptions, SyncSelector, plan_sync},
};

pub fn run(ctx: &RepoContext, worktree: Option<&str>, dry_run: bool) -> Result<ExitCode, WkError> {
    let config = load_config_or_default(ctx)?;
    let state = StateStore::new(&ctx.control_dir);
    let target = apply_target(ctx, worktree)?;
    let apply_plan = plan_apply(ctx, &config, &state, &target)?;
    execute_and_persist(&apply_plan, &state, dry_run)?;
    let sync_plan = plan_auto_sync(ctx, &config, &state, &target, dry_run)?;
    execute_and_persist(&sync_plan, &state, dry_run)?;
    Ok(ExitCode::SUCCESS)
}

fn plan_auto_sync(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    target: &ApplyTarget,
    dry_run: bool,
) -> Result<OperationPlan, WkError> {
    let mut plan = OperationPlan::default();
    for path_config in &config.paths {
        if path_config.mode != Mode::Sync
            || effective_sync_policy(config, path_config) != SyncPolicy::Auto
        {
            continue;
        }
        let selector = match target {
            ApplyTarget::All => SyncSelector::Path(path_config.path.clone()),
            ApplyTarget::Worktree(worktree_id) => {
                SyncSelector::PathAndWorktree(path_config.path.clone(), worktree_id.clone())
            }
        };
        plan.extend(plan_sync(
            ctx,
            config,
            state,
            &selector,
            SyncOptions {
                policy: SyncPolicy::Auto,
                conflict_policy: effective_conflict_policy(config, path_config),
                dry_run,
            },
        )?);
    }
    Ok(plan)
}

fn apply_target(ctx: &RepoContext, worktree: Option<&str>) -> Result<ApplyTarget, WkError> {
    let Some(selector) = worktree else {
        return Ok(ApplyTarget::All);
    };
    let selected = ctx
        .non_source_worktrees()
        .find(|info| info.id.as_str() == selector || info.path.as_str() == selector)
        .map(|info| info.id.clone())
        .ok_or_else(|| WkError::message(format!("unknown worktree: {selector}")))?;
    Ok(ApplyTarget::Worktree(selected))
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

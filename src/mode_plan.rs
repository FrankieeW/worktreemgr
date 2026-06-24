use crate::{
    config::Config,
    domain::{ManagedPath, Mode},
    error::WkError,
    fs_plan::FsOp,
    git_repo::RepoContext,
    materialize::{OperationPlan, StateUpdate, plan_link, plan_overlay_to_worktree},
    state::StateStore,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeOptions {
    pub dry_run: bool,
    pub choice: TransitionChoice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionChoice {
    Default,
    SourceWins,
    WorktreeWins,
    Skip,
}

pub fn plan_mode_change(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    path: &ManagedPath,
    target_mode: Mode,
    options: ModeOptions,
) -> Result<OperationPlan, WkError> {
    let current_mode = config
        .paths
        .iter()
        .find(|path_config| path_config.path == *path)
        .map_or(Mode::Ignore, |path_config| path_config.mode);
    let mut plan = OperationPlan::default();
    if current_mode == target_mode {
        plan.summary
            .push(format!("{path} already in {target_mode:?}"));
        return Ok(plan);
    }
    for worktree in ctx.non_source_worktrees() {
        plan.summary
            .push(format!("{path}: {current_mode:?} -> {target_mode:?}"));
        match (current_mode, target_mode, options.choice) {
            (Mode::Ignore, Mode::Ignore, _)
            | (Mode::Link, Mode::Link, _)
            | (Mode::Copy, Mode::Copy, _)
            | (Mode::Sync, Mode::Sync, _) => {}
            (Mode::Ignore | Mode::Copy, Mode::Link, _) => {
                plan.extend(plan_link_for_mode(ctx, state, path, worktree)?);
            }
            (_, Mode::Copy, _) | (_, Mode::Sync, TransitionChoice::SourceWins) => {
                plan.extend(plan_overlay_to_worktree(ctx, path, worktree)?);
            }
            (Mode::Link, Mode::Ignore, _) => {
                let dest = worktree.path.join(path.as_path());
                plan.fs_ops.push(FsOp::RemoveFile { path: dest });
                plan.extend(plan_overlay_to_worktree(ctx, path, worktree)?);
            }
            (_, Mode::Sync, _) => {
                plan.warnings
                    .push(format!("{path} requires sync initialization decision"));
            }
            (Mode::Sync, Mode::Link, _) => {
                plan.warnings
                    .push(format!("{path} sync -> link may discard worktree content"));
            }
            (_, Mode::Ignore, _) => {
                plan.state_updates.push(StateUpdate::Remove {
                    path: path.clone(),
                    worktree_id: worktree.id.clone(),
                });
            }
        }
    }
    if options.dry_run {
        plan.summary
            .push("dry run: no filesystem writes".to_owned());
    }
    Ok(plan)
}

fn plan_link_for_mode(
    ctx: &RepoContext,
    state: &StateStore,
    path: &ManagedPath,
    worktree: &crate::git_repo::WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    plan_link(ctx, path, state, worktree)
}

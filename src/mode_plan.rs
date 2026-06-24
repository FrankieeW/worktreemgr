use std::io::ErrorKind;

use crate::{
    config::Config,
    domain::{ManagedPath, Mode},
    error::WkError,
    fs_plan::{backup_path_op, plan_overlay_copy},
    git_repo::{RepoContext, WorktreeId, WorktreeInfo},
    materialize::{
        OperationPlan, StateUpdate, plan_copy_if_missing, plan_link, plan_overlay_to_worktree,
    },
    state::StateStore,
};

mod sync_transition;

use sync_transition::{plan_sync_default, plan_sync_source_wins, plan_sync_worktree_wins};

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
    plan.summary
        .push(format!("{path}: {current_mode:?} -> {target_mode:?}"));
    for worktree in ctx.non_source_worktrees() {
        match (current_mode, target_mode, options.choice) {
            (Mode::Ignore, Mode::Ignore, _)
            | (Mode::Link, Mode::Link, _)
            | (Mode::Copy, Mode::Copy, _)
            | (Mode::Sync, Mode::Sync, _) => {}
            (Mode::Ignore, Mode::Copy, _) => {
                plan.extend(plan_copy_if_missing(ctx, path, worktree)?);
            }
            (Mode::Link, Mode::Copy | Mode::Ignore, _) => {
                plan.extend(plan_link_to_copy(ctx, path, worktree)?);
                plan.state_updates.push(remove_state(path, &worktree.id));
            }
            (Mode::Ignore | Mode::Copy, Mode::Link, _) => {
                plan.extend(plan_link_for_mode(ctx, state, path, worktree)?);
            }
            (_, Mode::Sync, TransitionChoice::SourceWins) => {
                plan.extend(plan_sync_source_wins(ctx, path, worktree)?);
            }
            (_, Mode::Sync, TransitionChoice::WorktreeWins) => {
                plan.extend(plan_sync_worktree_wins(ctx, path, worktree)?);
            }
            (_, Mode::Sync, TransitionChoice::Default | TransitionChoice::Skip) => {
                plan.extend(plan_sync_default(ctx, path, worktree)?);
            }
            (Mode::Sync, Mode::Link, _) => {
                plan.warnings
                    .push(format!("{path} sync -> link skipped by default"));
            }
            (Mode::Sync, Mode::Copy, _) | (_, Mode::Ignore, _) => {
                plan.state_updates.push(remove_state(path, &worktree.id));
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

fn plan_link_to_copy(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let mut plan = OperationPlan::default();
    let dest = worktree.path.join(path.as_path());
    match std::fs::symlink_metadata(&dest) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            plan.fs_ops
                .push(backup_path_op(&dest, &ctx.control_dir.join("backups")));
            plan.extend(plan_overlay_to_worktree_after_root_backup(
                ctx, path, worktree,
            )?);
        }
        Ok(_metadata) => {
            plan.warnings.push(format!(
                "existing non-symlink destination left untouched: {dest}"
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            plan.extend(plan_overlay_to_worktree(ctx, path, worktree)?);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(plan)
}

fn plan_overlay_to_worktree_after_root_backup(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let source = ctx.main_worktree.join(path.as_path());
    let dest = worktree.path.join(path.as_path());
    Ok(OperationPlan {
        summary: vec![format!("overlay copy {source} -> {dest}")],
        fs_ops: plan_overlay_copy(&source, &dest)?,
        state_updates: Vec::new(),
        warnings: Vec::new(),
    })
}

fn remove_state(path: &ManagedPath, worktree_id: &WorktreeId) -> StateUpdate {
    StateUpdate::Remove {
        path: path.clone(),
        worktree_id: worktree_id.clone(),
    }
}

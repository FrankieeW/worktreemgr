use std::io::ErrorKind;

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    config::Config,
    domain::{ManagedPath, Mode},
    error::WkError,
    fs_plan::{backup_path_op, plan_overlay_copy, plan_overlay_copy_with_backups},
    git_repo::{RepoContext, WorktreeId, WorktreeInfo},
    manifest::{Manifest, build_manifest},
    materialize::{
        OperationPlan, StateUpdate, plan_copy_if_missing, plan_link, plan_overlay_to_worktree,
    },
    state::StateStore,
    state::{ConflictRecord, DestinationKind, MaterializationProvenance, PairStatus, PathState},
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

fn plan_sync_source_wins(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let mut plan = plan_overlay_to_worktree(ctx, path, worktree)?;
    let source_manifest = manifest_or_empty(&ctx.main_worktree.join(path.as_path()))?;
    plan.state_updates.push(clean_sync_state(
        path,
        &worktree.id,
        source_manifest.clone(),
        source_manifest,
    ));
    Ok(plan)
}

fn plan_sync_worktree_wins(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let source = worktree.path.join(path.as_path());
    let dest = ctx.main_worktree.join(path.as_path());
    let fs_ops = plan_overlay_copy_with_backups(&source, &dest, &ctx.control_dir.join("backups"))?;
    let worktree_manifest = manifest_or_empty(&source)?;
    Ok(OperationPlan {
        summary: vec![format!("overlay copy {source} -> {dest}")],
        fs_ops,
        state_updates: vec![clean_sync_state(
            path,
            &worktree.id,
            worktree_manifest.clone(),
            worktree_manifest,
        )],
        warnings: Vec::new(),
    })
}

fn plan_sync_default(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let source = ctx.main_worktree.join(path.as_path());
    let dest = worktree.path.join(path.as_path());
    if !dest_exists(&dest)? {
        return plan_sync_source_wins(ctx, path, worktree);
    }
    let source_manifest = manifest_or_empty(&source)?;
    let worktree_manifest = manifest_or_empty(&dest)?;
    if source_manifest.has_same_content_identity(&worktree_manifest) {
        return Ok(OperationPlan {
            summary: Vec::new(),
            fs_ops: Vec::new(),
            state_updates: vec![clean_sync_state(
                path,
                &worktree.id,
                source_manifest,
                worktree_manifest,
            )],
            warnings: Vec::new(),
        });
    }
    Ok(OperationPlan {
        summary: Vec::new(),
        fs_ops: Vec::new(),
        state_updates: vec![conflict_sync_state(
            path,
            &worktree.id,
            source_manifest,
            worktree_manifest,
        )],
        warnings: vec![format!("{path} differs; marked sync conflict")],
    })
}

fn clean_sync_state(
    path: &ManagedPath,
    worktree_id: &WorktreeId,
    source_manifest: Manifest,
    worktree_manifest: Manifest,
) -> StateUpdate {
    StateUpdate::Save(PathState {
        path: path.clone(),
        worktree_id: worktree_id.clone(),
        status: PairStatus::Clean,
        provenance: sync_provenance(),
        source_manifest: Some(source_manifest),
        worktree_manifest: Some(worktree_manifest),
        conflict: None,
    })
}

fn conflict_sync_state(
    path: &ManagedPath,
    worktree_id: &WorktreeId,
    source_manifest: Manifest,
    worktree_manifest: Manifest,
) -> StateUpdate {
    StateUpdate::Save(PathState {
        path: path.clone(),
        worktree_id: worktree_id.clone(),
        status: PairStatus::Conflict,
        provenance: sync_provenance(),
        source_manifest: Some(source_manifest),
        worktree_manifest: Some(worktree_manifest),
        conflict: Some(ConflictRecord {
            entries: vec![Utf8PathBuf::from("")],
            message: "mode transition requires sync conflict resolution".to_owned(),
        }),
    })
}

const fn sync_provenance() -> MaterializationProvenance {
    MaterializationProvenance {
        destination_kind: DestinationKind::SyncCopy,
        created_or_adopted_by_wk: true,
        expected_symlink_target: None,
    }
}

fn remove_state(path: &ManagedPath, worktree_id: &WorktreeId) -> StateUpdate {
    StateUpdate::Remove {
        path: path.clone(),
        worktree_id: worktree_id.clone(),
    }
}

fn dest_exists(path: &Utf8Path) -> Result<bool, WkError> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn manifest_or_empty(root: &Utf8Path) -> Result<Manifest, WkError> {
    match std::fs::symlink_metadata(root) {
        Ok(_metadata) => build_manifest(root),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(error.into()),
    }
}

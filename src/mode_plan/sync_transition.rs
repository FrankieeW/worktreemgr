use std::io::ErrorKind;

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    domain::ManagedPath,
    error::WkError,
    fs_plan::plan_overlay_copy_with_backups,
    git_repo::{RepoContext, WorktreeId, WorktreeInfo},
    manifest::{Manifest, build_manifest},
    materialize::{OperationPlan, StateUpdate, plan_overlay_to_worktree},
    state::{ConflictRecord, DestinationKind, MaterializationProvenance, PairStatus, PathState},
};

pub(super) fn plan_sync_source_wins(
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

pub(super) fn plan_sync_worktree_wins(
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

pub(super) fn plan_sync_default(
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

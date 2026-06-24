use std::io::ErrorKind;

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    config::{Config, PathConfig},
    domain::{ManagedPath, Mode},
    error::WkError,
    fs_plan::{FsOp, plan_overlay_copy},
    git_repo::{RepoContext, WorktreeId, WorktreeInfo},
    manifest::build_manifest,
    state::{DestinationKind, MaterializationProvenance, PairStatus, PathState, StateStore},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationPlan {
    pub summary: Vec<String>,
    pub fs_ops: Vec<FsOp>,
    pub state_updates: Vec<StateUpdate>,
    pub warnings: Vec<String>,
}

impl OperationPlan {
    pub fn extend(&mut self, other: Self) {
        self.summary.extend(other.summary);
        self.fs_ops.extend(other.fs_ops);
        self.state_updates.extend(other.state_updates);
        self.warnings.extend(other.warnings);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateUpdate {
    Save(PathState),
    Remove {
        path: ManagedPath,
        worktree_id: WorktreeId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyTarget {
    All,
    Worktree(WorktreeId),
}

pub fn plan_apply(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    target: &ApplyTarget,
) -> Result<OperationPlan, WkError> {
    let mut plan = OperationPlan::default();
    for path_config in &config.paths {
        for worktree in selected_worktrees(ctx, target) {
            plan.extend(plan_path_apply(ctx, path_config, state, worktree)?);
        }
    }
    Ok(plan)
}

pub(crate) fn plan_path_apply(
    ctx: &RepoContext,
    path_config: &PathConfig,
    state: &StateStore,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    match path_config.mode {
        Mode::Ignore => Ok(OperationPlan::default()),
        Mode::Link => plan_link(ctx, &path_config.path, state, worktree),
        Mode::Copy => plan_copy_if_missing(ctx, &path_config.path, worktree),
        Mode::Sync => plan_sync_copy_if_missing(ctx, path_config, worktree),
    }
}

pub(crate) fn plan_link(
    ctx: &RepoContext,
    path: &ManagedPath,
    state: &StateStore,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let source = ctx.main_worktree.join(path.as_path());
    let dest = worktree.path.join(path.as_path());
    let target = relative_symlink_target(&source, &dest)?;
    let mut plan = OperationPlan::default();
    match std::fs::symlink_metadata(&dest) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            plan.summary.push(format!("link {dest} -> {target}"));
            plan.fs_ops.push(FsOp::CopySymlink {
                target,
                dest,
                warning: None,
            });
        }
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let actual = read_link_utf8(&dest)?;
            if actual == target {
                return Ok(plan);
            }
            if can_repair_symlink(state, path, &worktree.id)? {
                plan.summary.push(format!("repair link {dest} -> {target}"));
                plan.fs_ops.push(FsOp::CopySymlink {
                    target,
                    dest,
                    warning: None,
                });
            } else {
                plan.warnings
                    .push(format!("foreign symlink left untouched: {dest}"));
            }
        }
        Ok(_metadata) => {
            plan.warnings.push(format!(
                "existing non-managed destination left untouched: {dest}"
            ));
        }
    }
    Ok(plan)
}

pub(crate) fn plan_overlay_to_worktree(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let source = ctx.main_worktree.join(path.as_path());
    let dest = worktree.path.join(path.as_path());
    let fs_ops = plan_overlay_copy(&source, &dest)?;
    Ok(OperationPlan {
        summary: vec![format!("overlay copy {source} -> {dest}")],
        fs_ops,
        state_updates: Vec::new(),
        warnings: Vec::new(),
    })
}

fn plan_copy_if_missing(
    ctx: &RepoContext,
    path: &ManagedPath,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let dest = worktree.path.join(path.as_path());
    if dest_exists(&dest)? {
        return Ok(OperationPlan::default());
    }
    plan_overlay_to_worktree(ctx, path, worktree)
}

fn plan_sync_copy_if_missing(
    ctx: &RepoContext,
    path_config: &PathConfig,
    worktree: &WorktreeInfo,
) -> Result<OperationPlan, WkError> {
    let mut plan = plan_copy_if_missing(ctx, &path_config.path, worktree)?;
    if !plan.fs_ops.is_empty() {
        let source = ctx.main_worktree.join(path_config.path.as_path());
        let source_manifest = build_manifest(&source)?;
        plan.state_updates.push(StateUpdate::Save(PathState {
            path: path_config.path.clone(),
            worktree_id: worktree.id.clone(),
            status: PairStatus::Clean,
            provenance: MaterializationProvenance {
                destination_kind: DestinationKind::SyncCopy,
                created_or_adopted_by_wk: true,
                expected_symlink_target: None,
            },
            source_manifest: Some(source_manifest.clone()),
            worktree_manifest: Some(source_manifest),
            conflict: None,
        }));
    }
    Ok(plan)
}

fn selected_worktrees<'a>(
    ctx: &'a RepoContext,
    target: &'a ApplyTarget,
) -> Box<dyn Iterator<Item = &'a WorktreeInfo> + 'a> {
    match target {
        ApplyTarget::All => Box::new(ctx.non_source_worktrees()),
        ApplyTarget::Worktree(id) => Box::new(
            ctx.non_source_worktrees()
                .filter(move |worktree| &worktree.id == id),
        ),
    }
}

fn can_repair_symlink(
    state: &StateStore,
    path: &ManagedPath,
    worktree_id: &WorktreeId,
) -> Result<bool, WkError> {
    let Some(path_state) = state.load_path_state(path, worktree_id)? else {
        return Ok(false);
    };
    Ok(
        path_state.provenance.destination_kind == DestinationKind::Symlink
            && path_state.provenance.created_or_adopted_by_wk,
    )
}

fn dest_exists(path: &Utf8Path) -> Result<bool, WkError> {
    match std::fs::symlink_metadata(path) {
        Ok(_metadata) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn relative_symlink_target(source: &Utf8Path, dest: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    let parent = dest
        .parent()
        .ok_or_else(|| WkError::message(format!("destination has no parent: {dest}")))?;
    let relative = pathdiff::diff_paths(source, parent).ok_or_else(|| {
        WkError::message(format!("cannot make relative link from {dest} to {source}"))
    })?;
    Utf8PathBuf::from_path_buf(relative)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

fn read_link_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    let target = std::fs::read_link(path)?;
    Utf8PathBuf::from_path_buf(target)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

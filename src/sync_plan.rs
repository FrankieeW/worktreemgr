use std::io::ErrorKind;

use camino::{Utf8Path, Utf8PathBuf};

mod entry;

use entry::{EntryPlan, NewerSide, SyncEntry};

use crate::{
    config::{Config, PathConfig},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    drift::{EntryDrift, classify_manifest_drift},
    error::WkError,
    git_repo::{RepoContext, WorktreeId, WorktreeInfo},
    manifest::{Manifest, build_manifest},
    materialize::{OperationPlan, StateUpdate},
    state::{
        ConflictRecord, DestinationKind, MaterializationProvenance, PairStatus, PathState,
        StateStore,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncOptions {
    pub policy: SyncPolicy,
    pub conflict_policy: ConflictPolicy,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncSelector {
    All,
    Path(ManagedPath),
    Worktree(WorktreeId),
    PathAndWorktree(ManagedPath, WorktreeId),
}

pub fn plan_sync(
    ctx: &RepoContext,
    config: &Config,
    state: &StateStore,
    selector: &SyncSelector,
    options: SyncOptions,
) -> Result<OperationPlan, WkError> {
    let mut plan = OperationPlan::default();
    push_option_summary(&mut plan, options);
    for path_config in config
        .paths
        .iter()
        .filter(|path_config| path_config.mode == Mode::Sync)
    {
        if !selector_matches_path(selector, &path_config.path) {
            continue;
        }
        for worktree in ctx.non_source_worktrees() {
            if selector_matches_worktree(selector, &worktree.id) {
                plan.extend(plan_pair_sync(ctx, path_config, state, worktree, options)?);
            }
        }
    }
    Ok(plan)
}

fn plan_pair_sync(
    ctx: &RepoContext,
    path_config: &PathConfig,
    state: &StateStore,
    worktree: &WorktreeInfo,
    options: SyncOptions,
) -> Result<OperationPlan, WkError> {
    let source_root = ctx.main_worktree.join(path_config.path.as_path());
    let worktree_root = worktree.path.join(path_config.path.as_path());
    let current_source = manifest_or_empty(&source_root)?;
    let current_worktree = manifest_or_empty(&worktree_root)?;
    let previous = load_or_initialize_state(state, &path_config.path, &worktree.id)?;
    let base_source = previous.source_manifest.clone().unwrap_or_default();
    let base_worktree = previous.worktree_manifest.clone().unwrap_or_default();
    let drift = classify_manifest_drift(
        &base_source,
        &base_worktree,
        &current_source,
        &current_worktree,
    );
    let backup_root = ctx.control_dir.join("backups");
    let mut entry_plan = EntryPlan::new(
        current_source.clone(),
        current_worktree.clone(),
        &backup_root,
    );

    for (entry_path, entry_drift) in &drift.entries {
        let sync_entry = SyncEntry {
            entry_path,
            source_root: &source_root,
            worktree_root: &worktree_root,
            current_source: &current_source,
            current_worktree: &current_worktree,
        };
        plan_entry_drift(
            &mut entry_plan,
            &sync_entry,
            *entry_drift,
            options.conflict_policy,
        )?;
    }

    let (fs_ops, expected_source, expected_worktree, conflicts, warnings) = entry_plan.finish();
    let next_state = next_pair_state(
        previous,
        expected_source,
        expected_worktree,
        conflicts,
        &base_source,
        &base_worktree,
    );
    Ok(OperationPlan {
        summary: Vec::new(),
        fs_ops,
        state_updates: vec![StateUpdate::Save(next_state)],
        warnings,
    })
}

fn plan_entry_drift(
    entry_plan: &mut EntryPlan,
    entry: &SyncEntry<'_>,
    drift: EntryDrift,
    policy: ConflictPolicy,
) -> Result<(), WkError> {
    match drift {
        EntryDrift::Unchanged | EntryDrift::BothChangedIdentically => Ok(()),
        EntryDrift::SourceAdded | EntryDrift::SourceModified => {
            entry_plan.source_to_worktree(entry)
        }
        EntryDrift::WorktreeAdded | EntryDrift::WorktreeModified => {
            entry_plan.worktree_to_source(entry)
        }
        EntryDrift::SourceDeleted => {
            entry_plan.delete_worktree_entry(entry);
            Ok(())
        }
        EntryDrift::WorktreeDeleted => {
            entry_plan.delete_source_entry(entry);
            Ok(())
        }
        EntryDrift::BothChangedDifferently => resolve_conflict(entry_plan, entry, policy),
    }
}

fn resolve_conflict(
    entry_plan: &mut EntryPlan,
    entry: &SyncEntry<'_>,
    policy: ConflictPolicy,
) -> Result<(), WkError> {
    match policy {
        ConflictPolicy::Ask => {
            entry_plan.mark_conflict(entry);
            Ok(())
        }
        ConflictPolicy::Source => entry_plan.source_to_worktree(entry),
        ConflictPolicy::Worktree => entry_plan.worktree_to_source(entry),
        ConflictPolicy::Newer => {
            entry_plan.warn_newer_policy(entry);
            match EntryPlan::newer_side(entry) {
                NewerSide::Source => entry_plan.source_to_worktree(entry),
                NewerSide::Worktree => entry_plan.worktree_to_source(entry),
                NewerSide::Unknown => {
                    entry_plan.mark_conflict(entry);
                    Ok(())
                }
            }
        }
    }
}

fn next_pair_state(
    previous: PathState,
    expected_source: Manifest,
    expected_worktree: Manifest,
    conflicts: Vec<Utf8PathBuf>,
    base_source: &Manifest,
    base_worktree: &Manifest,
) -> PathState {
    if conflicts.is_empty() {
        return PathState {
            status: PairStatus::Clean,
            source_manifest: Some(expected_source),
            worktree_manifest: Some(expected_worktree),
            conflict: None,
            ..previous
        };
    }
    PathState {
        status: PairStatus::Conflict,
        source_manifest: Some(base_source.clone()),
        worktree_manifest: Some(base_worktree.clone()),
        conflict: Some(ConflictRecord {
            entries: conflicts,
            message: "per-entry sync conflict".to_owned(),
        }),
        ..previous
    }
}

fn load_or_initialize_state(
    state: &StateStore,
    path: &ManagedPath,
    worktree_id: &WorktreeId,
) -> Result<PathState, WkError> {
    if let Some(existing) = state.load_path_state(path, worktree_id)? {
        return Ok(existing);
    }
    Ok(PathState {
        path: path.clone(),
        worktree_id: worktree_id.clone(),
        status: PairStatus::Uninitialized,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::SyncCopy,
            created_or_adopted_by_wk: true,
            expected_symlink_target: None,
        },
        source_manifest: Some(Manifest::default()),
        worktree_manifest: Some(Manifest::default()),
        conflict: None,
    })
}

fn manifest_or_empty(root: &Utf8Path) -> Result<Manifest, WkError> {
    match std::fs::symlink_metadata(root) {
        Ok(_metadata) => build_manifest(root),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(error.into()),
    }
}

fn selector_matches_path(selector: &SyncSelector, path: &ManagedPath) -> bool {
    match selector {
        SyncSelector::All | SyncSelector::Worktree(_) => true,
        SyncSelector::Path(selected) | SyncSelector::PathAndWorktree(selected, _) => {
            selected == path
        }
    }
}

fn selector_matches_worktree(selector: &SyncSelector, worktree_id: &WorktreeId) -> bool {
    match selector {
        SyncSelector::All | SyncSelector::Path(_) => true,
        SyncSelector::Worktree(selected) | SyncSelector::PathAndWorktree(_, selected) => {
            selected == worktree_id
        }
    }
}

fn push_option_summary(plan: &mut OperationPlan, options: SyncOptions) {
    if options.policy == SyncPolicy::Auto {
        plan.summary.push("auto sync".to_owned());
    }
    if options.dry_run {
        plan.summary
            .push("dry run: no filesystem writes".to_owned());
    }
}

use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    error::WkError,
    fs_plan::{FsOp, backup_path_op},
    manifest::{EntryKind, Manifest, ManifestEntry},
};

pub(super) struct SyncEntry<'a> {
    pub(super) entry_path: &'a Utf8PathBuf,
    pub(super) source_root: &'a Utf8Path,
    pub(super) worktree_root: &'a Utf8Path,
    pub(super) current_source: &'a Manifest,
    pub(super) current_worktree: &'a Manifest,
}

pub(super) struct EntryPlan<'a> {
    fs_ops: Vec<FsOp>,
    removals: Vec<FsOp>,
    expected_source: Manifest,
    expected_worktree: Manifest,
    conflicts: Vec<Utf8PathBuf>,
    warnings: Vec<String>,
    backup_root: &'a Utf8Path,
}

impl<'a> EntryPlan<'a> {
    pub(super) const fn new(
        current_source: Manifest,
        current_worktree: Manifest,
        backup_root: &'a Utf8Path,
    ) -> Self {
        Self {
            fs_ops: Vec::new(),
            removals: Vec::new(),
            expected_source: current_source,
            expected_worktree: current_worktree,
            conflicts: Vec::new(),
            warnings: Vec::new(),
            backup_root,
        }
    }

    pub(super) fn source_to_worktree(&mut self, entry: &SyncEntry<'_>) -> Result<(), WkError> {
        copy_or_delete_entry(
            entry.current_source.entries.get(entry.entry_path),
            entry.current_worktree.entries.get(entry.entry_path),
            entry.source_root,
            entry.worktree_root,
            entry.entry_path,
            &mut EntryBuffers {
                ops: &mut self.fs_ops,
                removals: &mut self.removals,
                expected: &mut self.expected_worktree,
                backup_root: self.backup_root,
            },
        )
    }

    pub(super) fn worktree_to_source(&mut self, entry: &SyncEntry<'_>) -> Result<(), WkError> {
        copy_or_delete_entry(
            entry.current_worktree.entries.get(entry.entry_path),
            entry.current_source.entries.get(entry.entry_path),
            entry.worktree_root,
            entry.source_root,
            entry.entry_path,
            &mut EntryBuffers {
                ops: &mut self.fs_ops,
                removals: &mut self.removals,
                expected: &mut self.expected_source,
                backup_root: self.backup_root,
            },
        )
    }

    pub(super) fn delete_worktree_entry(&mut self, entry: &SyncEntry<'_>) {
        if let Some(existing) = entry.current_worktree.entries.get(entry.entry_path) {
            self.removals.push(remove_entry_op(
                existing,
                &join_entry(entry.worktree_root, entry.entry_path),
                self.backup_root,
            ));
        }
        self.expected_worktree.entries.remove(entry.entry_path);
    }

    pub(super) fn delete_source_entry(&mut self, entry: &SyncEntry<'_>) {
        if let Some(existing) = entry.current_source.entries.get(entry.entry_path) {
            self.removals.push(remove_entry_op(
                existing,
                &join_entry(entry.source_root, entry.entry_path),
                self.backup_root,
            ));
        }
        self.expected_source.entries.remove(entry.entry_path);
    }

    pub(super) fn mark_conflict(&mut self, entry: &SyncEntry<'_>) {
        self.conflicts.push(entry.entry_path.clone());
        self.warnings.push(format!(
            "conflict requires resolution: {}",
            entry.entry_path
        ));
    }

    pub(super) fn warn_newer_policy(&mut self, entry: &SyncEntry<'_>) {
        self.warnings.push(format!(
            "newer conflict policy uses unsafe mtime comparison for {}",
            entry.entry_path
        ));
    }

    pub(super) fn newer_side(entry: &SyncEntry<'_>) -> NewerSide {
        match (
            entry.current_source.entries.get(entry.entry_path),
            entry.current_worktree.entries.get(entry.entry_path),
        ) {
            (Some(source), Some(worktree)) if source.mtime >= worktree.mtime => NewerSide::Source,
            (Some(_source), Some(_worktree)) => NewerSide::Worktree,
            _ => NewerSide::Unknown,
        }
    }

    pub(super) fn finish(
        mut self,
    ) -> (Vec<FsOp>, Manifest, Manifest, Vec<Utf8PathBuf>, Vec<String>) {
        self.removals.sort_by(remove_order);
        self.fs_ops.extend(self.removals);
        (
            self.fs_ops,
            self.expected_source,
            self.expected_worktree,
            self.conflicts,
            self.warnings,
        )
    }
}

pub(super) enum NewerSide {
    Source,
    Worktree,
    Unknown,
}

struct EntryBuffers<'a> {
    ops: &'a mut Vec<FsOp>,
    removals: &'a mut Vec<FsOp>,
    expected: &'a mut Manifest,
    backup_root: &'a Utf8Path,
}

fn copy_or_delete_entry(
    source_entry: Option<&ManifestEntry>,
    dest_entry: Option<&ManifestEntry>,
    source_root: &Utf8Path,
    dest_root: &Utf8Path,
    entry_path: &Utf8PathBuf,
    buffers: &mut EntryBuffers<'_>,
) -> Result<(), WkError> {
    if let Some(entry) = source_entry {
        if dest_entry.is_some_and(|existing| should_backup_before_copy(entry, existing)) {
            buffers.ops.push(backup_path_op(
                &join_entry(dest_root, entry_path),
                buffers.backup_root,
            ));
        }
        buffers
            .ops
            .push(copy_entry_op(entry, source_root, dest_root, entry_path)?);
        buffers
            .expected
            .entries
            .insert(entry_path.clone(), entry.clone());
        return Ok(());
    }
    if let Some(existing) = dest_entry {
        buffers.removals.push(remove_entry_op(
            existing,
            &join_entry(dest_root, entry_path),
            buffers.backup_root,
        ));
    }
    buffers.expected.entries.remove(entry_path);
    Ok(())
}

fn should_backup_before_copy(source: &ManifestEntry, dest: &ManifestEntry) -> bool {
    source.kind != EntryKind::Directory || dest.kind != EntryKind::Directory
}

fn copy_entry_op(
    entry: &ManifestEntry,
    source_root: &Utf8Path,
    dest_root: &Utf8Path,
    entry_path: &Utf8Path,
) -> Result<FsOp, WkError> {
    let source = join_entry(source_root, entry_path);
    let dest = join_entry(dest_root, entry_path);
    match entry.kind {
        EntryKind::Directory => Ok(FsOp::CreateDir { path: dest }),
        EntryKind::File => Ok(FsOp::CopyFile { source, dest }),
        EntryKind::Symlink => {
            let target = entry.target.clone().ok_or_else(|| {
                WkError::message(format!("symlink manifest entry has no target: {source}"))
            })?;
            Ok(FsOp::CopySymlink {
                warning: symlink_warning(&target),
                target,
                dest,
            })
        }
    }
}

fn remove_entry_op(entry: &ManifestEntry, path: &Utf8Path, backup_root: &Utf8Path) -> FsOp {
    match entry.kind {
        EntryKind::Directory | EntryKind::File | EntryKind::Symlink => {
            backup_path_op(path, backup_root)
        }
    }
}

fn join_entry(root: &Utf8Path, entry_path: &Utf8Path) -> Utf8PathBuf {
    if entry_path.as_str().is_empty() {
        return root.to_path_buf();
    }
    root.join(entry_path)
}

fn symlink_warning(target: &Utf8Path) -> Option<String> {
    if target.is_absolute() || target.as_str().split('/').any(|segment| segment == "..") {
        return Some(format!(
            "symlink target may point outside managed path: {target}"
        ));
    }
    None
}

fn remove_order(left: &FsOp, right: &FsOp) -> std::cmp::Ordering {
    remove_path(right)
        .components()
        .count()
        .cmp(&remove_path(left).components().count())
        .then_with(|| remove_path(right).cmp(remove_path(left)))
}

fn remove_path(op: &FsOp) -> &Utf8Path {
    match op {
        FsOp::RemoveFile { path }
        | FsOp::RemoveEmptyDir { path }
        | FsOp::BackupPath { path, .. } => path,
        FsOp::CreateDir { .. }
        | FsOp::CopyFile { .. }
        | FsOp::CopySymlink { .. }
        | FsOp::WriteFileAtomic { .. }
        | FsOp::SetPermissions { .. } => Utf8Path::new(""),
    }
}

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestEntry};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DriftReport {
    pub entries: BTreeMap<Utf8PathBuf, EntryDrift>,
}

impl DriftReport {
    pub fn has_conflict(&self) -> bool {
        self.entries
            .values()
            .any(|drift| *drift == EntryDrift::BothChangedDifferently)
    }

    pub fn is_clean_after_refresh(&self) -> bool {
        self.entries.values().all(|drift| {
            matches!(
                drift,
                EntryDrift::Unchanged | EntryDrift::BothChangedIdentically
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryDrift {
    Unchanged,
    SourceAdded,
    WorktreeAdded,
    SourceModified,
    WorktreeModified,
    SourceDeleted,
    WorktreeDeleted,
    BothChangedIdentically,
    BothChangedDifferently,
}

pub fn classify_manifest_drift(
    base_source: &Manifest,
    base_worktree: &Manifest,
    current_source: &Manifest,
    current_worktree: &Manifest,
) -> DriftReport {
    let mut entries = BTreeMap::new();
    for path in manifest_paths(base_source, base_worktree, current_source, current_worktree) {
        let drift = classify_entry(
            base_source.entries.get(&path),
            base_worktree.entries.get(&path),
            current_source.entries.get(&path),
            current_worktree.entries.get(&path),
        );
        entries.insert(path, drift);
    }
    DriftReport { entries }
}

fn manifest_paths(
    base_source: &Manifest,
    base_worktree: &Manifest,
    current_source: &Manifest,
    current_worktree: &Manifest,
) -> BTreeSet<Utf8PathBuf> {
    base_source
        .entries
        .keys()
        .chain(base_worktree.entries.keys())
        .chain(current_source.entries.keys())
        .chain(current_worktree.entries.keys())
        .cloned()
        .collect()
}

fn classify_entry(
    base_source: Option<&ManifestEntry>,
    base_worktree: Option<&ManifestEntry>,
    current_source: Option<&ManifestEntry>,
    current_worktree: Option<&ManifestEntry>,
) -> EntryDrift {
    let source_changed = !same_entry_identity(current_source, base_source);
    let worktree_changed = !same_entry_identity(current_worktree, base_worktree);
    match (source_changed, worktree_changed) {
        (false, false) => EntryDrift::Unchanged,
        (true, false) => source_only_drift(base_source, current_source),
        (false, true) => worktree_only_drift(base_worktree, current_worktree),
        (true, true) if same_entry_identity(current_source, current_worktree) => {
            EntryDrift::BothChangedIdentically
        }
        (true, true) => EntryDrift::BothChangedDifferently,
    }
}

fn same_entry_identity(left: Option<&ManifestEntry>, right: Option<&ManifestEntry>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.has_same_content_identity(right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

const fn source_only_drift(
    base_source: Option<&ManifestEntry>,
    current_source: Option<&ManifestEntry>,
) -> EntryDrift {
    match (base_source, current_source) {
        (None, Some(_entry)) => EntryDrift::SourceAdded,
        (Some(_entry), None) => EntryDrift::SourceDeleted,
        (Some(_old), Some(_new)) => EntryDrift::SourceModified,
        (None, None) => EntryDrift::Unchanged,
    }
}

const fn worktree_only_drift(
    base_worktree: Option<&ManifestEntry>,
    current_worktree: Option<&ManifestEntry>,
) -> EntryDrift {
    match (base_worktree, current_worktree) {
        (None, Some(_entry)) => EntryDrift::WorktreeAdded,
        (Some(_entry), None) => EntryDrift::WorktreeDeleted,
        (Some(_old), Some(_new)) => EntryDrift::WorktreeModified,
        (None, None) => EntryDrift::Unchanged,
    }
}

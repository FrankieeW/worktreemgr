use std::collections::BTreeMap;

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::tempdir;
use wk::{
    domain::ManagedPath,
    drift::{EntryDrift, classify_manifest_drift},
    git_repo::{WorktreeId, WorktreeInfo},
    manifest::{EntryKind, Manifest, ManifestEntry},
    state::{
        ConflictRecord, DestinationKind, MaterializationProvenance, PairStatus, PathState,
        StateStore,
    },
};

#[test]
fn state_roundtrips_manifest_and_symlink_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let control_dir = utf8_path(temp.path())?.join(".wk");
    let store = StateStore::new(&control_dir);
    let state = sample_path_state(PairStatus::Clean, None)?;

    store.save_path_state(&state)?;
    let loaded = store
        .load_path_state(&state.path, &state.worktree_id)?
        .ok_or("missing saved state")?;

    assert_eq!(loaded, state);
    assert_eq!(
        loaded.provenance.expected_symlink_target.as_deref(),
        Some(Utf8Path::new("../main/.claude"))
    );
    Ok(())
}

#[test]
fn source_worktree_does_not_create_destination_state() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let control_dir = utf8_path(temp.path())?.join(".wk");
    let store = StateStore::new(&control_dir);
    let state = sample_path_state(PairStatus::Clean, None)?;
    let source = WorktreeInfo {
        id: WorktreeId::main(),
        path: utf8_path(temp.path())?.to_path_buf(),
        is_source: true,
    };

    store.save_for_worktree(&source, &state)?;

    assert!(
        store
            .load_path_state(&state.path, &state.worktree_id)?
            .is_none()
    );
    Ok(())
}

#[test]
fn classifies_every_per_entry_drift_variant() {
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        "a",
        EntryDrift::Unchanged,
    );
    assert_drift(
        &Manifest::default(),
        &Manifest::default(),
        &manifest([("a", "one")]),
        &Manifest::default(),
        "a",
        EntryDrift::SourceAdded,
    );
    assert_drift(
        &Manifest::default(),
        &Manifest::default(),
        &Manifest::default(),
        &manifest([("a", "one")]),
        "a",
        EntryDrift::WorktreeAdded,
    );
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "two")]),
        &manifest([("a", "one")]),
        "a",
        EntryDrift::SourceModified,
    );
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "two")]),
        "a",
        EntryDrift::WorktreeModified,
    );
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &Manifest::default(),
        &manifest([("a", "one")]),
        "a",
        EntryDrift::SourceDeleted,
    );
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &Manifest::default(),
        "a",
        EntryDrift::WorktreeDeleted,
    );
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "two")]),
        &manifest([("a", "two")]),
        "a",
        EntryDrift::BothChangedIdentically,
    );
    assert_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "two")]),
        &manifest([("a", "three")]),
        "a",
        EntryDrift::BothChangedDifferently,
    );
}

#[test]
fn conflict_persists_until_convergence() -> Result<(), Box<dyn std::error::Error>> {
    let conflict = ConflictRecord {
        entries: vec![Utf8PathBuf::from("a")],
        message: "both changed".to_owned(),
    };
    let state = sample_path_state(PairStatus::Conflict, Some(conflict))?;
    let conflicting = classify_manifest_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &manifest([("a", "two")]),
        &manifest([("a", "three")]),
    );
    let still_conflict = state.clone().refresh_after_status(
        &conflicting,
        manifest([("a", "two")]),
        manifest([("a", "three")]),
    );

    assert_eq!(still_conflict.status, PairStatus::Conflict);
    assert!(still_conflict.conflict.is_some());

    let converged_source = manifest([("a", "two")]);
    let converged_worktree = manifest([("a", "two")]);
    let converged = classify_manifest_drift(
        &manifest([("a", "one")]),
        &manifest([("a", "one")]),
        &converged_source,
        &converged_worktree,
    );
    let clean = state.refresh_after_status(&converged, converged_source, converged_worktree);

    assert_eq!(clean.status, PairStatus::Clean);
    assert!(clean.conflict.is_none());
    Ok(())
}

fn assert_drift(
    base_source: &Manifest,
    base_worktree: &Manifest,
    current_source: &Manifest,
    current_worktree: &Manifest,
    path: &str,
    expected: EntryDrift,
) {
    let report =
        classify_manifest_drift(base_source, base_worktree, current_source, current_worktree);
    assert_eq!(report.entries[Utf8Path::new(path)], expected);
}

fn sample_path_state(
    status: PairStatus,
    conflict: Option<ConflictRecord>,
) -> Result<PathState, Box<dyn std::error::Error>> {
    Ok(PathState {
        path: ManagedPath::parse(".claude")?,
        worktree_id: WorktreeId::linked("feature"),
        status,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::Symlink,
            created_or_adopted_by_wk: true,
            expected_symlink_target: Some(Utf8PathBuf::from("../main/.claude")),
        },
        source_manifest: Some(manifest([("settings.json", "one")])),
        worktree_manifest: Some(manifest([("settings.json", "one")])),
        conflict,
    })
}

fn manifest<const N: usize>(entries: [(&str, &str); N]) -> Manifest {
    Manifest {
        entries: entries
            .into_iter()
            .map(|(path, hash)| (Utf8PathBuf::from(path), file_entry(hash)))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn file_entry(hash: &str) -> ManifestEntry {
    ManifestEntry {
        kind: EntryKind::File,
        hash: Some(hash.to_owned()),
        target: None,
        executable: false,
        size: 1,
        mtime: 1,
    }
}

fn utf8_path(path: &std::path::Path) -> Result<&Utf8Path, Box<dyn std::error::Error>> {
    Utf8Path::from_path(path).ok_or_else(|| "temporary path is not UTF-8".into())
}

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{
    atomic::{ensure_private_dir, write_file_atomic},
    domain::ManagedPath,
    drift::DriftReport,
    error::WkError,
    git_repo::{WorktreeId, WorktreeInfo},
    manifest::Manifest,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateStore {
    state_dir: Utf8PathBuf,
}

impl StateStore {
    pub fn new(control_dir: &Utf8Path) -> Self {
        Self {
            state_dir: control_dir.join("state"),
        }
    }

    pub fn save_path_state(&self, state: &PathState) -> Result<(), WkError> {
        let file = self.state_file(&state.path, &state.worktree_id);
        if let Some(parent) = file.parent() {
            ensure_private_dir(parent)?;
        }
        let contents = serde_json::to_vec_pretty(state)?;
        write_file_atomic(&file, &contents)
    }

    pub fn save_for_worktree(
        &self,
        worktree: &WorktreeInfo,
        state: &PathState,
    ) -> Result<(), WkError> {
        if worktree.is_source {
            return Ok(());
        }
        self.save_path_state(state)
    }

    pub fn load_path_state(
        &self,
        path: &ManagedPath,
        worktree_id: &WorktreeId,
    ) -> Result<Option<PathState>, WkError> {
        let file = self.state_file(path, worktree_id);
        if !file.exists() {
            return Ok(None);
        }
        let contents = std::fs::read(&file)?;
        Ok(Some(serde_json::from_slice(&contents)?))
    }

    fn state_file(&self, path: &ManagedPath, worktree_id: &WorktreeId) -> Utf8PathBuf {
        self.state_dir
            .join(worktree_id.as_str())
            .join(format!("{}.json", encode_path(path)))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PathState {
    pub path: ManagedPath,
    pub worktree_id: WorktreeId,
    pub status: PairStatus,
    pub provenance: MaterializationProvenance,
    pub source_manifest: Option<Manifest>,
    pub worktree_manifest: Option<Manifest>,
    pub conflict: Option<ConflictRecord>,
}

impl PathState {
    #[must_use]
    pub fn refresh_after_status(
        mut self,
        report: &DriftReport,
        current_source: Manifest,
        current_worktree: Manifest,
    ) -> Self {
        if report.is_clean_after_refresh() {
            self.status = PairStatus::Clean;
            self.conflict = None;
            self.source_manifest = Some(current_source);
            self.worktree_manifest = Some(current_worktree);
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairStatus {
    Uninitialized,
    Clean,
    Conflict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKind {
    Missing,
    Copy,
    SyncCopy,
    Symlink,
    Foreign,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MaterializationProvenance {
    pub destination_kind: DestinationKind,
    pub created_or_adopted_by_wk: bool,
    pub expected_symlink_target: Option<Utf8PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConflictRecord {
    pub entries: Vec<Utf8PathBuf>,
    pub message: String,
}

fn encode_path(path: &ManagedPath) -> String {
    path.as_str()
        .chars()
        .map(|character| match character {
            '/' => '+',
            '.' => '_',
            other => other,
        })
        .collect()
}

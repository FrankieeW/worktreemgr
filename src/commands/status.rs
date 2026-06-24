use std::{io::ErrorKind, process::ExitCode};

use camino::Utf8Path;
use serde::Serialize;

use crate::{
    commands::load_config_or_default,
    domain::{ManagedPath, Mode},
    drift::classify_manifest_drift,
    error::WkError,
    git_repo::{RepoContext, WorktreeId},
    manifest::{Manifest, build_manifest},
    state::{PairStatus, StateStore},
};

#[derive(Serialize)]
struct StatusOutput {
    rows: Vec<StatusRow>,
}

#[derive(Serialize)]
struct StatusRow {
    path: String,
    worktree_id: String,
    mode: Mode,
    status: RowStatus,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RowStatus {
    Clean,
    Drift,
    Conflict,
    Uninitialized,
}

pub fn run(ctx: &RepoContext, json: bool) -> Result<ExitCode, WkError> {
    let config = load_config_or_default(ctx)?;
    let state = StateStore::new(&ctx.control_dir);
    let mut rows = Vec::new();
    for path_config in &config.paths {
        for worktree in ctx.non_source_worktrees() {
            rows.push(status_row(
                ctx,
                &state,
                &path_config.path,
                path_config.mode,
                &worktree.id,
                &worktree.path,
            )?);
        }
    }
    let code = exit_code(&rows);
    if json {
        serde_json::to_writer(std::io::stdout().lock(), &StatusOutput { rows })?;
        println!();
    } else {
        for row in rows {
            println!(
                "{}\t{}\t{:?}\t{}",
                row.path,
                row.worktree_id,
                row.mode,
                status_label(row.status)
            );
        }
    }
    Ok(code)
}

fn status_row(
    ctx: &RepoContext,
    state: &StateStore,
    path: &ManagedPath,
    mode: Mode,
    worktree_id: &WorktreeId,
    worktree_root: &Utf8Path,
) -> Result<StatusRow, WkError> {
    let status = if mode == Mode::Sync {
        sync_status(ctx, state, path, worktree_id, worktree_root)?
    } else {
        non_sync_status(worktree_root.join(path.as_path()).exists())
    };
    Ok(StatusRow {
        path: path.as_str().to_owned(),
        worktree_id: worktree_id.as_str().to_owned(),
        mode,
        status,
    })
}

fn sync_status(
    ctx: &RepoContext,
    state: &StateStore,
    path: &ManagedPath,
    worktree_id: &WorktreeId,
    worktree_root: &Utf8Path,
) -> Result<RowStatus, WkError> {
    let Some(path_state) = state.load_path_state(path, worktree_id)? else {
        return Ok(RowStatus::Uninitialized);
    };
    if path_state.status == PairStatus::Conflict {
        return Ok(RowStatus::Conflict);
    }
    let base_source = path_state.source_manifest.unwrap_or_default();
    let base_worktree = path_state.worktree_manifest.unwrap_or_default();
    let current_source = manifest_or_empty(&ctx.main_worktree.join(path.as_path()))?;
    let current_worktree = manifest_or_empty(&worktree_root.join(path.as_path()))?;
    let drift = classify_manifest_drift(
        &base_source,
        &base_worktree,
        &current_source,
        &current_worktree,
    );
    if drift.has_conflict() {
        return Ok(RowStatus::Conflict);
    }
    if drift.is_clean_after_refresh() {
        return Ok(RowStatus::Clean);
    }
    Ok(RowStatus::Drift)
}

const fn non_sync_status(exists: bool) -> RowStatus {
    if exists {
        RowStatus::Clean
    } else {
        RowStatus::Uninitialized
    }
}

fn manifest_or_empty(root: &Utf8Path) -> Result<Manifest, WkError> {
    match std::fs::symlink_metadata(root) {
        Ok(_metadata) => build_manifest(root),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Manifest::default()),
        Err(error) => Err(error.into()),
    }
}

fn exit_code(rows: &[StatusRow]) -> ExitCode {
    if rows
        .iter()
        .any(|row| matches!(row.status, RowStatus::Conflict))
    {
        return ExitCode::from(2);
    }
    if rows
        .iter()
        .any(|row| !matches!(row.status, RowStatus::Clean))
    {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

const fn status_label(status: RowStatus) -> &'static str {
    match status {
        RowStatus::Clean => "clean",
        RowStatus::Drift => "drift",
        RowStatus::Conflict => "conflict",
        RowStatus::Uninitialized => "uninitialized",
    }
}

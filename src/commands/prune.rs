use std::{collections::BTreeSet, process::ExitCode};

use camino::Utf8Path;

use crate::{error::WkError, git_repo::RepoContext};

pub fn run(ctx: &RepoContext) -> Result<ExitCode, WkError> {
    let state_dir = ctx.control_dir.join("state");
    if !state_dir.exists() {
        return Ok(ExitCode::SUCCESS);
    }
    let live_ids = ctx
        .non_source_worktrees()
        .map(|worktree| worktree.id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    for entry in std::fs::read_dir(&state_dir)? {
        let entry = entry?;
        let path = camino_path(&entry.path())?;
        if path.is_dir() && stale_state_dir(&path, &live_ids) {
            println!("prune state {}", path.file_name().unwrap_or_default());
            std::fs::remove_dir_all(path)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn stale_state_dir(path: &Utf8Path, live_ids: &BTreeSet<String>) -> bool {
    path.file_name()
        .is_some_and(|name| !live_ids.contains(name))
}

fn camino_path(path: &std::path::Path) -> Result<camino::Utf8PathBuf, WkError> {
    camino::Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

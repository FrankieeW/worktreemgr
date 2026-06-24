use std::{
    process::ExitCode,
    time::{Duration, SystemTime},
};

use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;

use crate::{error::WkError, git_repo::RepoContext};

#[derive(Clone, Debug, Eq, PartialEq)]
struct BackupCandidate {
    path: Utf8PathBuf,
    modified: SystemTime,
}

pub fn run(
    ctx: &RepoContext,
    force: bool,
    older_than: Option<&str>,
    keep: Option<usize>,
) -> Result<ExitCode, WkError> {
    let backups = ctx.control_dir.join("backups");
    if !backups.exists() {
        return Ok(ExitCode::SUCCESS);
    }
    let threshold = parse_duration(older_than.unwrap_or("30d"))?;
    let keep = keep.unwrap_or(0);
    let mut candidates = old_backup_files(&backups, threshold)?;
    candidates.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.path.cmp(&left.path))
    });
    for candidate in candidates.into_iter().skip(keep) {
        println!("{}", candidate.path);
        if force {
            std::fs::remove_file(candidate.path)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn old_backup_files(root: &Utf8Path, threshold: Duration) -> Result<Vec<BackupCandidate>, WkError> {
    let cutoff = SystemTime::now()
        .checked_sub(threshold)
        .ok_or_else(|| WkError::message("backup retention duration is too large".to_owned()))?;
    let mut candidates = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = camino_path(entry.path())?;
        let modified = entry.metadata()?.modified()?;
        if modified <= cutoff {
            candidates.push(BackupCandidate { path, modified });
        }
    }
    Ok(candidates)
}

fn parse_duration(raw: &str) -> Result<Duration, WkError> {
    let days = raw
        .strip_suffix('d')
        .ok_or_else(|| WkError::message(format!("unsupported duration: {raw}")))?
        .parse::<u64>()
        .map_err(|error| WkError::message(error.to_string()))?;
    Ok(Duration::from_secs(days * 24 * 60 * 60))
}

fn camino_path(path: &std::path::Path) -> Result<Utf8PathBuf, WkError> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

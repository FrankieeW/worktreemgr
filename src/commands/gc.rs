use std::{
    process::ExitCode,
    time::{Duration, SystemTime},
};

use camino::{Utf8Path, Utf8PathBuf};

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
    dry_run: bool,
) -> Result<ExitCode, WkError> {
    let backups = ctx.control_dir.join("backups");
    if !backups.exists() {
        return Ok(ExitCode::SUCCESS);
    }
    let threshold = parse_duration(older_than.unwrap_or("30d"))?;
    let keep = keep.unwrap_or(0);
    let mut candidates = old_backup_units(&backups, threshold)?;
    candidates.sort_by(|left, right| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| right.path.cmp(&left.path))
    });
    for candidate in candidates.into_iter().skip(keep) {
        println!("{}", candidate.path);
        if force && !dry_run {
            remove_backup_unit(&candidate.path)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn old_backup_units(root: &Utf8Path, threshold: Duration) -> Result<Vec<BackupCandidate>, WkError> {
    let cutoff = SystemTime::now()
        .checked_sub(threshold)
        .ok_or_else(|| WkError::message("backup retention duration is too large".to_owned()))?;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !(file_type.is_file() || file_type.is_dir() || file_type.is_symlink()) {
            continue;
        }
        let path = camino_path(&entry.path())?;
        let modified = entry.metadata()?.modified()?;
        if modified <= cutoff {
            candidates.push(BackupCandidate { path, modified });
        }
    }
    Ok(candidates)
}

fn remove_backup_unit(path: &Utf8Path) -> Result<(), WkError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

fn parse_duration(raw: &str) -> Result<Duration, WkError> {
    let (amount, unit) = raw.split_at(raw.len().saturating_sub(1));
    let amount = amount
        .parse::<u64>()
        .map_err(|error| WkError::message(error.to_string()))?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => return Err(WkError::message(format!("unsupported duration: {raw}"))),
    };
    amount
        .checked_mul(seconds)
        .map(Duration::from_secs)
        .ok_or_else(|| WkError::message(format!("duration is too large: {raw}")))
}

fn camino_path(path: &std::path::Path) -> Result<Utf8PathBuf, WkError> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

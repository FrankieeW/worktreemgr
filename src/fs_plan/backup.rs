use std::time::{Duration, SystemTime, UNIX_EPOCH};

use camino::{Utf8Path, Utf8PathBuf};

use crate::{atomic::ensure_private_dir, error::WkError};

use super::FsOp;

pub(super) fn backup_path_op(path: &Utf8Path, backup_root: &Utf8Path) -> FsOp {
    FsOp::BackupPath {
        path: path.to_path_buf(),
        backup: backup_path(path, backup_root),
    }
}

pub(super) fn execute_backup(path: &Utf8Path, backup: &Utf8Path) -> Result<(), WkError> {
    ensure_backup_parent(backup)?;
    std::fs::rename(path, backup)?;
    set_backup_permissions(backup)
}

fn ensure_backup_parent(path: &Utf8Path) -> Result<(), WkError> {
    let parent = path
        .parent()
        .ok_or_else(|| WkError::message(format!("backup path has no parent: {path}")))?;
    ensure_private_dir(parent)
}

fn set_backup_permissions(path: &Utf8Path) -> Result<(), WkError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_file() {
        return set_permissions(path, 0o600);
    }
    if metadata.file_type().is_dir() {
        return set_permissions(path, 0o700);
    }
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Utf8Path, mode: u32) -> Result<(), WkError> {
    use std::os::unix::fs::PermissionsExt as _;

    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_permissions(_path: &Utf8Path, _mode: u32) -> Result<(), WkError> {
    Ok(())
}

fn backup_path(path: &Utf8Path, backup_root: &Utf8Path) -> Utf8PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, duration_nanos);
    backup_root.join(format!("{timestamp}-{}", sanitize_path(path)))
}

fn duration_nanos(duration: Duration) -> u128 {
    u128::from(duration.as_secs()) * 1_000_000_000_u128 + u128::from(duration.subsec_nanos())
}

fn sanitize_path(path: &Utf8Path) -> String {
    let sanitized = path
        .as_str()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        return "root".to_owned();
    }
    sanitized
}

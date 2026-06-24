use camino::{Utf8Path, Utf8PathBuf};
use walkdir::{DirEntry, WalkDir};

mod backup;
mod execute;

pub use execute::{ExecutionReport, execute_plan};

use crate::error::WkError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FsOp {
    CreateDir {
        path: Utf8PathBuf,
    },
    CopyFile {
        source: Utf8PathBuf,
        dest: Utf8PathBuf,
    },
    CopySymlink {
        target: Utf8PathBuf,
        dest: Utf8PathBuf,
        warning: Option<String>,
    },
    RemoveFile {
        path: Utf8PathBuf,
    },
    RemoveEmptyDir {
        path: Utf8PathBuf,
    },
    BackupPath {
        path: Utf8PathBuf,
        backup: Utf8PathBuf,
    },
    WriteFileAtomic {
        path: Utf8PathBuf,
        contents: Vec<u8>,
    },
    SetPermissions {
        path: Utf8PathBuf,
        mode: u32,
    },
}

pub fn plan_overlay_copy(source: &Utf8Path, dest: &Utf8Path) -> Result<Vec<FsOp>, WkError> {
    plan_overlay_copy_inner(source, dest, None)
}

pub fn plan_overlay_copy_with_backups(
    source: &Utf8Path,
    dest: &Utf8Path,
    backup_root: &Utf8Path,
) -> Result<Vec<FsOp>, WkError> {
    plan_overlay_copy_inner(source, dest, Some(backup_root))
}

pub(crate) fn backup_path_op(path: &Utf8Path, backup_root: &Utf8Path) -> FsOp {
    backup::backup_path_op(path, backup_root)
}

fn plan_overlay_copy_inner(
    source: &Utf8Path,
    dest: &Utf8Path,
    backup_root: Option<&Utf8Path>,
) -> Result<Vec<FsOp>, WkError> {
    let metadata = std::fs::symlink_metadata(source)?;
    let mut ops = Vec::new();
    if metadata.is_dir() {
        ops.push(FsOp::CreateDir {
            path: dest.to_path_buf(),
        });
        for entry in WalkDir::new(source).follow_links(false) {
            let entry = entry?;
            let relative = relative_entry_path(source, &entry)?;
            if relative.as_str().is_empty() {
                continue;
            }
            push_copy_op(&mut ops, &entry, &dest.join(relative), backup_root)?;
        }
    } else {
        push_copy_path_op(&mut ops, source, dest, backup_root)?;
    }
    Ok(ops)
}

fn push_copy_op(
    ops: &mut Vec<FsOp>,
    entry: &DirEntry,
    dest: &Utf8Path,
    backup_root: Option<&Utf8Path>,
) -> Result<(), WkError> {
    let source = Utf8Path::from_path(entry.path())
        .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?;
    push_copy_path_op(ops, source, dest, backup_root)
}

fn push_copy_path_op(
    ops: &mut Vec<FsOp>,
    source: &Utf8Path,
    dest: &Utf8Path,
    backup_root: Option<&Utf8Path>,
) -> Result<(), WkError> {
    let metadata = std::fs::symlink_metadata(source)?;
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        ops.push(FsOp::CreateDir {
            path: dest.to_path_buf(),
        });
        return Ok(());
    }
    if file_type.is_symlink() {
        let target = read_link_utf8(source)?;
        push_backup_if_replacing(ops, dest, backup_root)?;
        ops.push(FsOp::CopySymlink {
            warning: symlink_warning(&target),
            target,
            dest: dest.to_path_buf(),
        });
        return Ok(());
    }
    if file_type.is_file() {
        push_backup_if_replacing(ops, dest, backup_root)?;
        ops.push(FsOp::CopyFile {
            source: source.to_path_buf(),
            dest: dest.to_path_buf(),
        });
        return Ok(());
    }
    Err(WkError::message(format!(
        "unsupported filesystem entry for overlay copy: {source}"
    )))
}

fn read_link_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    let target = std::fs::read_link(path)?;
    Utf8PathBuf::from_path_buf(target)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

fn symlink_warning(target: &Utf8Path) -> Option<String> {
    if target.is_absolute() || target.as_str().split('/').any(|segment| segment == "..") {
        return Some(format!(
            "symlink target may point outside managed path: {target}"
        ));
    }
    None
}

fn push_backup_if_replacing(
    ops: &mut Vec<FsOp>,
    dest: &Utf8Path,
    backup_root: Option<&Utf8Path>,
) -> Result<(), WkError> {
    let Some(root) = backup_root else {
        return Ok(());
    };
    match std::fs::symlink_metadata(dest) {
        Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
            ops.push(backup_path_op(dest, root));
        }
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_metadata) => {
            return Err(WkError::message(format!(
                "unsupported destination entry for backup: {dest}"
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn relative_entry_path<'a>(root: &Utf8Path, entry: &'a DirEntry) -> Result<&'a Utf8Path, WkError> {
    let path = Utf8Path::from_path(entry.path())
        .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?;
    path.strip_prefix(root)
        .map_err(|error| WkError::message(error.to_string()))
}

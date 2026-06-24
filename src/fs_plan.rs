use std::io::Write as _;

use camino::{Utf8Path, Utf8PathBuf};
use walkdir::{DirEntry, WalkDir};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReport {
    pub operations: Vec<String>,
    pub warnings: Vec<String>,
    pub dry_run: bool,
}

pub fn plan_overlay_copy(source: &Utf8Path, dest: &Utf8Path) -> Result<Vec<FsOp>, WkError> {
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
            push_copy_op(&mut ops, &entry, &dest.join(relative))?;
        }
    } else {
        push_copy_path_op(&mut ops, source, dest)?;
    }
    Ok(ops)
}

pub fn execute_plan(ops: &[FsOp], dry_run: bool) -> Result<ExecutionReport, WkError> {
    let mut report = ExecutionReport {
        operations: Vec::with_capacity(ops.len()),
        warnings: Vec::new(),
        dry_run,
    };
    for op in ops {
        report.operations.push(operation_summary(op));
        if let FsOp::CopySymlink {
            warning: Some(warning),
            ..
        } = op
        {
            report.warnings.push(warning.clone());
        }
        if !dry_run {
            execute_op(op)?;
        }
    }
    Ok(report)
}

fn push_copy_op(ops: &mut Vec<FsOp>, entry: &DirEntry, dest: &Utf8Path) -> Result<(), WkError> {
    let source = Utf8Path::from_path(entry.path())
        .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?;
    push_copy_path_op(ops, source, dest)
}

fn push_copy_path_op(
    ops: &mut Vec<FsOp>,
    source: &Utf8Path,
    dest: &Utf8Path,
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
        ops.push(FsOp::CopySymlink {
            warning: symlink_warning(&target),
            target,
            dest: dest.to_path_buf(),
        });
        return Ok(());
    }
    if file_type.is_file() {
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

fn execute_op(op: &FsOp) -> Result<(), WkError> {
    match op {
        FsOp::CreateDir { path } => std::fs::create_dir_all(path)?,
        FsOp::CopyFile { source, dest } => copy_file(source, dest)?,
        FsOp::CopySymlink { target, dest, .. } => copy_symlink(target, dest)?,
        FsOp::RemoveFile { path } => std::fs::remove_file(path)?,
        FsOp::RemoveEmptyDir { path } => std::fs::remove_dir(path)?,
        FsOp::BackupPath { path, backup } => {
            ensure_parent(backup)?;
            std::fs::rename(path, backup)?;
        }
        FsOp::WriteFileAtomic { path, contents } => write_file_atomic(path, contents)?,
        FsOp::SetPermissions { path, mode } => set_permissions(path, *mode)?,
    }
    Ok(())
}

fn copy_file(source: &Utf8Path, dest: &Utf8Path) -> Result<(), WkError> {
    ensure_parent(dest)?;
    if let Ok(metadata) = std::fs::symlink_metadata(dest) {
        if metadata.is_dir() {
            return Err(WkError::message(format!(
                "cannot replace directory with file during overlay copy: {dest}"
            )));
        }
        std::fs::remove_file(dest)?;
    }
    std::fs::copy(source, dest)?;
    Ok(())
}

fn copy_symlink(target: &Utf8Path, dest: &Utf8Path) -> Result<(), WkError> {
    ensure_parent(dest)?;
    if let Ok(metadata) = std::fs::symlink_metadata(dest) {
        if metadata.is_dir() {
            return Err(WkError::message(format!(
                "cannot replace directory with symlink during overlay copy: {dest}"
            )));
        }
        std::fs::remove_file(dest)?;
    }
    create_symlink(target, dest)
}

#[cfg(unix)]
fn create_symlink(target: &Utf8Path, dest: &Utf8Path) -> Result<(), WkError> {
    std::os::unix::fs::symlink(target, dest)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_symlink(_target: &Utf8Path, _dest: &Utf8Path) -> Result<(), WkError> {
    Err(WkError::message(
        "symlink copy is not implemented on this platform".to_owned(),
    ))
}

fn write_file_atomic(path: &Utf8Path, contents: &[u8]) -> Result<(), WkError> {
    let parent = path
        .parent()
        .ok_or_else(|| WkError::message(format!("path has no parent: {path}")))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|error| WkError::Persist {
        path: path.to_path_buf(),
        source: error,
    })?;
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

fn ensure_parent(path: &Utf8Path) -> Result<(), WkError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
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

fn operation_summary(op: &FsOp) -> String {
    match op {
        FsOp::CreateDir { path } => format!("create dir {path}"),
        FsOp::CopyFile { source, dest } => format!("copy file {source} -> {dest}"),
        FsOp::CopySymlink { target, dest, .. } => {
            format!("copy symlink {dest} -> {target}")
        }
        FsOp::RemoveFile { path } => format!("remove file {path}"),
        FsOp::RemoveEmptyDir { path } => format!("remove empty dir {path}"),
        FsOp::BackupPath { path, backup } => format!("backup {path} -> {backup}"),
        FsOp::WriteFileAtomic { path, .. } => format!("write file {path}"),
        FsOp::SetPermissions { path, mode } => format!("chmod {mode:o} {path}"),
    }
}

fn relative_entry_path<'a>(root: &Utf8Path, entry: &'a DirEntry) -> Result<&'a Utf8Path, WkError> {
    let path = Utf8Path::from_path(entry.path())
        .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?;
    path.strip_prefix(root)
        .map_err(|error| WkError::message(error.to_string()))
}

use std::io::Write as _;

use camino::Utf8Path;

use crate::error::WkError;

use super::{FsOp, backup};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReport {
    pub operations: Vec<String>,
    pub warnings: Vec<String>,
    pub dry_run: bool,
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

fn execute_op(op: &FsOp) -> Result<(), WkError> {
    match op {
        FsOp::CreateDir { path } => std::fs::create_dir_all(path)?,
        FsOp::CopyFile { source, dest } => copy_file(source, dest)?,
        FsOp::CopySymlink { target, dest, .. } => copy_symlink(target, dest)?,
        FsOp::RemoveFile { path } => std::fs::remove_file(path)?,
        FsOp::RemoveEmptyDir { path } => std::fs::remove_dir(path)?,
        FsOp::BackupPath { path, backup } => backup::execute_backup(path, backup)?,
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

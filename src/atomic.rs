use std::io::Write as _;

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::WkError;

pub fn write_file_atomic(path: &Utf8Path, contents: &[u8]) -> Result<(), WkError> {
    ensure_parent(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| WkError::message(format!("path has no parent: {path}")))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    temp.as_file_mut().sync_all()?;
    set_private_file_std(temp.path())?;
    temp.persist(path).map_err(|error| WkError::Persist {
        path: path.to_path_buf(),
        source: error,
    })?;
    set_private_file(path)?;
    Ok(())
}

pub fn ensure_private_dir(path: &Utf8Path) -> Result<(), WkError> {
    std::fs::create_dir_all(path)?;
    set_private_dir(path)?;
    Ok(())
}

fn ensure_parent(path: &Utf8Path) -> Result<(), WkError> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    Ok(())
}

fn set_private_file(path: &Utf8Path) -> Result<(), WkError> {
    set_private_file_std(path.as_std_path())
}

#[cfg(unix)]
fn set_private_file_std(path: &std::path::Path) -> Result<(), WkError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_std(_path: &std::path::Path) -> Result<(), WkError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_dir(path: &Utf8Path) -> Result<(), WkError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_dir(_path: &Utf8Path) -> Result<(), WkError> {
    Ok(())
}

pub fn utf8_pathbuf(path: std::path::PathBuf) -> Result<Utf8PathBuf, WkError> {
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

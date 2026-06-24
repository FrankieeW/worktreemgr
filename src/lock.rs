use std::{fs::File, fs::OpenOptions};

use camino::{Utf8Path, Utf8PathBuf};
use fs4::fs_std::FileExt as _;

use crate::{atomic::ensure_private_dir, error::WkError};

#[derive(Debug)]
pub struct MutationLock {
    file: File,
    _path: Utf8PathBuf,
}

impl MutationLock {
    pub fn acquire(control_dir: &Utf8Path) -> Result<Self, WkError> {
        ensure_private_dir(control_dir)?;
        let path = control_dir.join("lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        set_lock_permissions(&path)?;
        if !file.try_lock_exclusive()? {
            return Err(WkError::lock_busy(path));
        }
        Ok(Self { file, _path: path })
    }
}

impl Drop for MutationLock {
    fn drop(&mut self) {
        let _ = fs4::fs_std::FileExt::unlock(&self.file);
    }
}

pub fn read_only_state_dir(control_dir: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    Ok(control_dir.join("state"))
}

#[cfg(unix)]
fn set_lock_permissions(path: &Utf8Path) -> Result<(), WkError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_lock_permissions(_path: &Utf8Path) -> Result<(), WkError> {
    Ok(())
}

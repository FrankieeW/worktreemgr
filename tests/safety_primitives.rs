use camino::Utf8Path;
use tempfile::tempdir;
use wk::{
    atomic::{ensure_private_dir, write_file_atomic},
    lock::{MutationLock, read_only_state_dir},
};

#[test]
fn atomic_write_replaces_file_and_uses_private_file_mode() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    let path = root.join("state.json");
    std::fs::write(&path, "old")?;

    write_file_atomic(&path, b"new")?;

    assert_eq!(std::fs::read_to_string(&path)?, "new");
    #[cfg(unix)]
    assert_eq!(file_mode(&path)?, 0o600);
    Ok(())
}

#[test]
fn private_dir_is_created_with_owner_only_mode() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    let wk_dir = root.join(".wk");

    ensure_private_dir(&wk_dir)?;

    assert!(wk_dir.is_dir());
    #[cfg(unix)]
    assert_eq!(file_mode(&wk_dir)?, 0o700);
    Ok(())
}

#[test]
fn mutation_lock_rejects_second_mutation_lock() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let control_dir = utf8_path(temp.path())?.join(".wk");
    ensure_private_dir(&control_dir)?;
    let _first = MutationLock::acquire(&control_dir)?;

    let second = MutationLock::acquire(&control_dir).expect_err("second lock must fail");

    assert!(second.to_string().contains("lock"));
    Ok(())
}

#[test]
fn read_only_status_snapshot_is_not_blocked_by_mutation_lock()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let control_dir = utf8_path(temp.path())?.join(".wk");
    ensure_private_dir(&control_dir)?;
    let _lock = MutationLock::acquire(&control_dir)?;

    let state_dir = read_only_state_dir(&control_dir)?;

    assert_eq!(state_dir, control_dir.join("state"));
    Ok(())
}

#[cfg(unix)]
fn file_mode(path: &Utf8Path) -> Result<u32, Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt as _;

    Ok(std::fs::metadata(path)?.permissions().mode() & 0o777)
}

fn utf8_path(path: &std::path::Path) -> Result<&Utf8Path, Box<dyn std::error::Error>> {
    Utf8Path::from_path(path).ok_or_else(|| "temporary path is not UTF-8".into())
}

use camino::Utf8Path;
use tempfile::tempdir;
use wk::manifest::{EntryKind, Manifest, ManifestEntry, build_manifest};

#[test]
fn manifest_records_directory_entries_and_excludes_reserved_dirs()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    std::fs::write(root.join("file.txt"), "hello")?;
    std::fs::create_dir(root.join("empty"))?;
    std::fs::create_dir(root.join(".git"))?;
    std::fs::write(root.join(".git/config"), "git")?;
    std::fs::create_dir(root.join(".wk"))?;
    std::fs::write(root.join(".wk/state"), "wk")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let executable = root.join("run.sh");
        std::fs::write(&executable, "#!/bin/sh\n")?;
        let mut permissions = std::fs::metadata(&executable)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions)?;
        std::os::unix::fs::symlink("file.txt", root.join("link-to-file"))?;
    }

    let manifest = build_manifest(root)?;

    assert_eq!(entry(&manifest, "file.txt")?.kind, EntryKind::File);
    assert_eq!(entry(&manifest, "empty")?.kind, EntryKind::Directory);
    assert!(
        entry(&manifest, "file.txt")?
            .hash
            .as_deref()
            .is_some_and(|hash| hash.len() == 64)
    );
    assert!(!manifest.entries.contains_key(Utf8Path::new(".git/config")));
    assert!(!manifest.entries.contains_key(Utf8Path::new(".wk/state")));
    #[cfg(unix)]
    {
        assert!(entry(&manifest, "run.sh")?.executable);
        assert_eq!(entry(&manifest, "link-to-file")?.kind, EntryKind::Symlink);
        assert_eq!(
            entry(&manifest, "link-to-file")?.target.as_deref(),
            Some(Utf8Path::new("file.txt"))
        );
    }
    Ok(())
}

#[test]
fn manifest_enumerates_adds_and_deletes_even_when_parent_mtime_is_unchanged()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    std::fs::write(root.join("before.txt"), "before")?;
    let root_mtime = filetime::FileTime::from_last_modification_time(&std::fs::metadata(root)?);

    let before = build_manifest(root)?;
    std::fs::remove_file(root.join("before.txt"))?;
    std::fs::write(root.join("after.txt"), "after")?;
    filetime::set_file_mtime(root, root_mtime)?;
    let after = build_manifest(root)?;

    assert!(before.entries.contains_key(Utf8Path::new("before.txt")));
    assert!(!after.entries.contains_key(Utf8Path::new("before.txt")));
    assert!(after.entries.contains_key(Utf8Path::new("after.txt")));
    Ok(())
}

fn entry<'a>(
    manifest: &'a Manifest,
    path: &str,
) -> Result<&'a ManifestEntry, Box<dyn std::error::Error>> {
    manifest
        .entries
        .get(Utf8Path::new(path))
        .ok_or_else(|| format!("missing manifest entry: {path}").into())
}

fn utf8_path(path: &std::path::Path) -> Result<&Utf8Path, Box<dyn std::error::Error>> {
    Utf8Path::from_path(path).ok_or_else(|| "temporary path is not UTF-8".into())
}

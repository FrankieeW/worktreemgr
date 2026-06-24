use camino::Utf8Path;
use tempfile::tempdir;
use wk::fs_plan::{execute_plan, plan_overlay_copy};

#[test]
fn overlay_copy_preserves_destination_only_files() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir_all(&source)?;
    std::fs::create_dir_all(&dest)?;
    std::fs::write(source.join("from-source.txt"), "source")?;
    std::fs::write(dest.join("dest-only.txt"), "dest")?;

    let plan = plan_overlay_copy(&source, &dest)?;
    execute_plan(&plan, false)?;

    assert_eq!(
        std::fs::read_to_string(dest.join("from-source.txt"))?,
        "source"
    );
    assert_eq!(std::fs::read_to_string(dest.join("dest-only.txt"))?, "dest");
    Ok(())
}

#[cfg(unix)]
#[test]
fn overlay_copy_preserves_symlinks_without_dereferencing() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir_all(&source)?;
    std::fs::write(source.join("target.txt"), "target")?;
    std::os::unix::fs::symlink("target.txt", source.join("link"))?;

    let plan = plan_overlay_copy(&source, &dest)?;
    execute_plan(&plan, false)?;

    assert!(
        std::fs::symlink_metadata(dest.join("link"))?
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_link(dest.join("link"))?,
        std::path::PathBuf::from("target.txt")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn out_of_tree_symlink_is_preserved_and_warned() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir_all(&source)?;
    std::os::unix::fs::symlink("../outside.txt", source.join("outside-link"))?;

    let plan = plan_overlay_copy(&source, &dest)?;
    let report = execute_plan(&plan, true)?;

    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("outside"))
    );
    assert!(!dest.join("outside-link").exists());
    Ok(())
}

#[test]
fn dry_run_reports_operations_and_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let root = utf8_path(temp.path())?;
    let source = root.join("source");
    let dest = root.join("dest");
    std::fs::create_dir_all(&source)?;
    std::fs::write(source.join("file.txt"), "source")?;

    let plan = plan_overlay_copy(&source, &dest)?;
    let report = execute_plan(&plan, true)?;

    assert!(report.dry_run);
    assert!(!report.operations.is_empty());
    assert!(!dest.join("file.txt").exists());
    Ok(())
}

fn utf8_path(path: &std::path::Path) -> Result<&Utf8Path, Box<dyn std::error::Error>> {
    Utf8Path::from_path(path).ok_or_else(|| "temporary path is not UTF-8".into())
}

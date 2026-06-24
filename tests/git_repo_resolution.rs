use camino::{Utf8Path, Utf8PathBuf};
use tempfile::{TempDir, tempdir};
use wk::git_repo::discover_repo;

#[test]
fn resolves_same_control_store_from_main_and_linked_worktree()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;

    let from_main = discover_repo(&fixture.main)?;
    let from_linked = discover_repo(&fixture.linked)?;

    assert_eq!(from_main.control_dir, fixture.main.join(".wk"));
    assert_eq!(from_main.control_dir, from_linked.control_dir);
    assert_eq!(from_main.git_common_dir, fixture.main.join(".git"));
    assert_eq!(from_linked.git_common_dir, fixture.main.join(".git"));
    Ok(())
}

#[test]
fn non_source_worktrees_excludes_main_worktree() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    let context = discover_repo(&fixture.linked)?;
    let destinations = context.non_source_worktrees().collect::<Vec<_>>();

    assert_eq!(destinations.len(), 1);
    assert_eq!(destinations[0].path, fixture.linked);
    assert!(!destinations[0].is_source);
    Ok(())
}

#[test]
fn outside_git_returns_repository_error() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let path = utf8_path(temp.path())?;

    let error = discover_repo(path).expect_err("outside git must fail");

    assert!(error.to_string().contains("git repository"));
    Ok(())
}

struct GitFixture {
    _temp: TempDir,
    main: Utf8PathBuf,
    linked: Utf8PathBuf,
}

impl GitFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let root = utf8_path(temp.path())?.to_path_buf();
        let main_requested = root.join("main");
        let linked_requested = root.join("feature");
        std::fs::create_dir(&main_requested)?;
        let main = canonical_utf8(&main_requested)?;

        git(&main, ["init", "-b", "main"])?;
        git(&main, ["config", "user.email", "wk@example.invalid"])?;
        git(&main, ["config", "user.name", "wk test"])?;
        std::fs::write(main.join("README.md"), "fixture\n")?;
        git(&main, ["add", "README.md"])?;
        git(&main, ["commit", "-m", "initial"])?;
        git(
            &main,
            [
                "worktree",
                "add",
                "-b",
                "feature",
                linked_requested.as_str(),
            ],
        )?;
        let linked = canonical_utf8(&linked_requested)?;

        Ok(Self {
            _temp: temp,
            main,
            linked,
        })
    }
}

fn git<const N: usize>(cwd: &Utf8Path, args: [&str; N]) -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!("git failed: {}", String::from_utf8_lossy(&output.stderr)).into())
}

fn utf8_path(path: &std::path::Path) -> Result<&Utf8Path, Box<dyn std::error::Error>> {
    Utf8Path::from_path(path).ok_or_else(|| "temporary path is not UTF-8".into())
}

fn canonical_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, Box<dyn std::error::Error>> {
    Utf8PathBuf::from_path_buf(std::fs::canonicalize(path)?)
        .map_err(|path| format!("path is not UTF-8: {}", path.display()).into())
}

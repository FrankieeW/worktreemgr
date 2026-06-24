use camino::{Utf8Path, Utf8PathBuf};
use tempfile::{TempDir, tempdir};
use wk::{
    discovery::{
        DiscoveryOptions, DiscoverySource, discover_candidates, ensure_manageable_ignored,
        expand_explicit_or_glob,
    },
    domain::ManagedPath,
    git_repo::discover_repo,
};

#[test]
fn default_discovery_finds_ignored_ai_and_local_paths() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_ignore(".claude/\n.codex/\nAGENTS.local.md\ndocs/local/\n*.local.*\n.wk/\n")?;
    fixture.write_file(".claude/settings.json", "{}")?;
    fixture.write_file(".codex/config.toml", "")?;
    fixture.write_file("AGENTS.local.md", "agent")?;
    fixture.write_file("docs/local/note.md", "note")?;
    let context = discover_repo(&fixture.main)?;

    let discovered = discover_candidates(
        &context,
        DiscoveryOptions {
            include_defaults: true,
            force: false,
        },
    )?;
    let paths = discovered
        .iter()
        .map(|item| item.path.as_str())
        .collect::<Vec<_>>();

    assert!(paths.contains(&".claude"));
    assert!(paths.contains(&".codex"));
    assert!(paths.contains(&"AGENTS.local.md"));
    assert!(paths.contains(&"docs/local"));
    assert!(discovered.iter().all(|item| item.ignored));
    Ok(())
}

#[test]
fn glob_expansion_returns_concrete_ignored_paths() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_ignore("*.local.*\n")?;
    fixture.write_file("AGENTS.local.md", "agent")?;
    fixture.write_file("CODEX.local.md", "codex")?;
    let context = discover_repo(&fixture.main)?;

    let discovered = expand_explicit_or_glob(&context, "*.local.*", false)?;
    let paths = discovered
        .iter()
        .map(|item| item.path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(paths, vec!["AGENTS.local.md", "CODEX.local.md"]);
    assert!(
        discovered
            .iter()
            .all(|item| item.source == DiscoverySource::Glob)
    );
    assert!(paths.iter().all(|path| !path.contains('*')));
    Ok(())
}

#[test]
fn non_ignored_explicit_path_is_rejected_without_force() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file("README.md", "readme")?;
    let context = discover_repo(&fixture.main)?;
    let path = ManagedPath::parse("README.md")?;

    let error = ensure_manageable_ignored(&context, &path, false).expect_err("must reject");

    assert!(error.to_string().contains("not ignored"));
    Ok(())
}

#[test]
fn force_allows_explicit_non_ignored_path() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file("README.md", "readme")?;
    let context = discover_repo(&fixture.main)?;

    let discovered = expand_explicit_or_glob(&context, "README.md", true)?;

    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].path.as_str(), "README.md");
    assert!(!discovered[0].ignored);
    assert!(discovered[0].forced);
    Ok(())
}

#[test]
fn discovery_never_returns_git_or_wk_paths() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_ignore(".wk/\n.git/\n")?;
    fixture.write_file(".wk/state/file.json", "{}")?;
    let context = discover_repo(&fixture.main)?;

    let wk_matches = expand_explicit_or_glob(&context, ".wk/*", true)?;
    let git_matches = expand_explicit_or_glob(&context, ".git/*", true)?;

    assert!(wk_matches.is_empty());
    assert!(git_matches.is_empty());
    Ok(())
}

struct GitFixture {
    _temp: TempDir,
    main: Utf8PathBuf,
}

impl GitFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let root = utf8_path(temp.path())?.to_path_buf();
        let main_requested = root.join("main");
        std::fs::create_dir(&main_requested)?;
        let main = canonical_utf8(&main_requested)?;
        git(&main, ["init", "-b", "main"])?;
        Ok(Self { _temp: temp, main })
    }

    fn write_ignore(&self, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::write(self.main.join(".gitignore"), contents)?;
        Ok(())
    }

    fn write_file(&self, path: &str, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        let full_path = self.main.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(full_path, contents)?;
        Ok(())
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

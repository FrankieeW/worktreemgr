use camino::{Utf8Path, Utf8PathBuf};
use tempfile::{TempDir, tempdir};
use wk::{
    config::{Config, PathConfig},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    manifest::Manifest,
    state::{DestinationKind, MaterializationProvenance, PairStatus, PathState, StateStore},
};

pub fn sync_config() -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: vec![PathConfig {
            path: ManagedPath::parse("docs/local")?,
            mode: Mode::Sync,
            sync_policy: Some(SyncPolicy::Manual),
            conflict_policy: Some(ConflictPolicy::Ask),
        }],
    })
}

pub fn save_clean_state(
    state: &StateStore,
    context: &wk::git_repo::RepoContext,
    path: &str,
    source_manifest: Manifest,
    worktree_manifest: Manifest,
) -> Result<(), Box<dyn std::error::Error>> {
    let worktree = context
        .non_source_worktrees()
        .next()
        .ok_or("missing worktree")?;
    state.save_path_state(&PathState {
        path: ManagedPath::parse(path)?,
        worktree_id: worktree.id.clone(),
        status: PairStatus::Clean,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::SyncCopy,
            created_or_adopted_by_wk: true,
            expected_symlink_target: None,
        },
        source_manifest: Some(source_manifest),
        worktree_manifest: Some(worktree_manifest),
        conflict: None,
    })?;
    Ok(())
}

pub fn empty_manifest() -> Manifest {
    Manifest::default()
}

pub struct GitFixture {
    _temp: TempDir,
    pub main: Utf8PathBuf,
    pub linked: Utf8PathBuf,
}

impl GitFixture {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
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

    pub fn write_file(&self, path: &str, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        write_file(&self.main, path, contents)
    }

    pub fn write_linked_file(
        &self,
        path: &str,
        contents: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        write_file(&self.linked, path, contents)
    }
}

fn write_file(
    root: &Utf8Path,
    path: &str,
    contents: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let full_path = root.join(path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(full_path, contents)?;
    Ok(())
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

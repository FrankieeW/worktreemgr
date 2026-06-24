use camino::{Utf8Path, Utf8PathBuf};
use wk::{
    atomic::ensure_private_dir,
    config::{Config, PathConfig, save_config_atomic},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
};

use crate::support::GitFixture;

pub fn save_config(
    context: &wk::git_repo::RepoContext,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_private_dir(&context.control_dir)?;
    save_config_atomic(&context.control_dir.join("config.toml"), config)?;
    Ok(())
}

pub fn mixed_config() -> Result<Config, wk::error::WkError> {
    Ok(Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: vec![
            PathConfig {
                path: ManagedPath::parse(".claude")?,
                mode: Mode::Link,
                sync_policy: None,
                conflict_policy: None,
            },
            PathConfig {
                path: ManagedPath::parse("AGENTS.local.md")?,
                mode: Mode::Copy,
                sync_policy: None,
                conflict_policy: None,
            },
            sync_path_config()?,
        ],
    })
}

pub fn link_config() -> Result<Config, wk::error::WkError> {
    Ok(Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: vec![PathConfig {
            path: ManagedPath::parse(".claude")?,
            mode: Mode::Link,
            sync_policy: None,
            conflict_policy: None,
        }],
    })
}

pub fn copy_config() -> Result<Config, wk::error::WkError> {
    Ok(Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: vec![PathConfig {
            path: ManagedPath::parse("AGENTS.local.md")?,
            mode: Mode::Copy,
            sync_policy: None,
            conflict_policy: None,
        }],
    })
}

pub const fn empty_config() -> Config {
    Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: Vec::new(),
    }
}

pub fn add_worktree(
    fixture: &GitFixture,
    name: &str,
) -> Result<Utf8PathBuf, Box<dyn std::error::Error>> {
    let worktree = fixture.main.parent().ok_or("missing temp root")?.join(name);
    git(
        &fixture.main,
        ["worktree", "add", "-b", name, worktree.as_str()],
    )?;
    canonical_utf8(&worktree)
}

fn sync_path_config() -> Result<PathConfig, wk::error::WkError> {
    Ok(PathConfig {
        path: ManagedPath::parse("docs/local")?,
        mode: Mode::Sync,
        sync_policy: Some(SyncPolicy::Manual),
        conflict_policy: Some(ConflictPolicy::Ask),
    })
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

fn canonical_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, Box<dyn std::error::Error>> {
    Utf8PathBuf::from_path_buf(std::fs::canonicalize(path)?)
        .map_err(|path| format!("path is not UTF-8: {}", path.display()).into())
}

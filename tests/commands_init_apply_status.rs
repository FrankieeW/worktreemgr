mod support;

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use std::process::ExitCode;
use support::{GitFixture, empty_manifest, save_clean_state, sync_config};
use wk::{
    atomic::ensure_private_dir,
    cli::{Cli, Command},
    commands::run_command,
    config::{Config, PathConfig, save_config_atomic},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    git_repo::discover_repo,
    manifest::build_manifest,
    state::{
        ConflictRecord, DestinationKind, MaterializationProvenance, PairStatus, PathState,
        StateStore,
    },
    ui::Prompter,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn init_creates_control_store_and_gitignore_idempotently() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file(".gitignore", "*.local.*\n")?;
    fixture.write_file("AGENTS.local.md", "local agents")?;
    let prompter = FixedPrompter::new(Mode::Link);

    let first = run_command(cli(Command::Init), &fixture.main, &prompter)?;
    let second = run_command(cli(Command::Init), &fixture.main, &prompter)?;

    assert_eq!(first, ExitCode::SUCCESS);
    assert_eq!(second, ExitCode::SUCCESS);
    assert!(fixture.main.join(".wk").exists());
    let gitignore = std::fs::read_to_string(fixture.main.join(".gitignore"))?;
    assert_eq!(gitignore.matches(".wk/").count(), 1);
    let config = wk::config::load_config(&fixture.main.join(".wk/config.toml"))?;
    assert_eq!(config.paths.len(), 1);
    assert_eq!(config.paths[0].path, ManagedPath::parse("AGENTS.local.md")?);
    assert_eq!(config.paths[0].mode, Mode::Link);
    Ok(())
}

#[test]
fn add_glob_expands_to_concrete_config_entries() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file(".gitignore", "*.local.*\n")?;
    fixture.write_file("A.local.md", "a")?;
    fixture.write_file("B.local.md", "b")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &Config::default_for_tests())?;

    let exit = run_command(
        cli(Command::Add {
            path: "*.local.*".to_owned(),
            force: false,
        }),
        &fixture.main,
        &FixedPrompter::new(Mode::Copy),
    )?;

    assert_eq!(exit, ExitCode::SUCCESS);
    let config = wk::config::load_config(&context.control_dir.join("config.toml"))?;
    let paths = config
        .paths
        .iter()
        .map(|path_config| path_config.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["A.local.md", "B.local.md"]);
    assert!(
        config
            .paths
            .iter()
            .all(|path_config| path_config.mode == Mode::Copy)
    );
    Ok(())
}

#[test]
fn apply_dry_run_prints_plan_and_writes_nothing() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    let context = discover_repo(&fixture.main)?;
    save_config(
        &context,
        &Config {
            version: 1,
            default_sync_policy: SyncPolicy::Manual,
            default_conflict_policy: ConflictPolicy::Ask,
            paths: vec![PathConfig {
                path: ManagedPath::parse("docs/local")?,
                mode: Mode::Copy,
                sync_policy: None,
                conflict_policy: None,
            }],
        },
    )?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["--dry-run", "apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("copy file"));
    assert!(!fixture.linked.join("docs/local/source.md").exists());
    Ok(())
}

#[test]
fn apply_auto_sync_invokes_sync_planning() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    fixture.write_linked_file("docs/local/.keep", "keep")?;
    let context = discover_repo(&fixture.main)?;
    save_config(
        &context,
        &Config {
            version: 1,
            default_sync_policy: SyncPolicy::Manual,
            default_conflict_policy: ConflictPolicy::Ask,
            paths: vec![PathConfig {
                path: ManagedPath::parse("docs/local")?,
                mode: Mode::Sync,
                sync_policy: Some(SyncPolicy::Auto),
                conflict_policy: Some(ConflictPolicy::Ask),
            }],
        },
    )?;
    let state = StateStore::new(&context.control_dir);
    save_clean_state(
        &state,
        &context,
        "docs/local",
        empty_manifest(),
        empty_manifest(),
    )?;

    let exit = run_command(
        cli(Command::Apply { worktree: None }),
        &fixture.main,
        &FixedPrompter::new(Mode::Sync),
    )?;

    assert_eq!(exit, ExitCode::SUCCESS);
    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/source.md"))?,
        "source"
    );
    Ok(())
}

#[test]
fn status_json_exit_codes_distinguish_clean_drift_and_conflict() -> TestResult {
    let clean = GitFixture::new()?;
    write_sync_config_and_clean_state(&clean, "same", "same")?;
    AssertCommand::cargo_bin("wk")?
        .current_dir(&clean.main)
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\":\"clean\""));

    let drift = GitFixture::new()?;
    write_sync_config_and_clean_state(&drift, "base", "base")?;
    drift.write_file("docs/local/shared.md", "source")?;
    AssertCommand::cargo_bin("wk")?
        .current_dir(&drift.main)
        .args(["status", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"status\":\"drift\""));

    let conflict = GitFixture::new()?;
    write_sync_config_and_clean_state(&conflict, "base", "base")?;
    conflict.write_file("docs/local/shared.md", "source")?;
    conflict.write_linked_file("docs/local/shared.md", "worktree")?;
    AssertCommand::cargo_bin("wk")?
        .current_dir(&conflict.main)
        .args(["status", "--json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("\"status\":\"conflict\""));
    Ok(())
}

#[test]
fn status_reports_copy_drift() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("AGENTS.local.md", "source")?;
    fixture.write_linked_file("AGENTS.local.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    save_config(
        &context,
        &Config {
            version: 1,
            default_sync_policy: SyncPolicy::Manual,
            default_conflict_policy: ConflictPolicy::Ask,
            paths: vec![PathConfig {
                path: ManagedPath::parse("AGENTS.local.md")?,
                mode: Mode::Copy,
                sync_policy: None,
                conflict_policy: None,
            }],
        },
    )?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["status", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"mode\":\"copy\""))
        .stdout(predicate::str::contains("\"status\":\"drift\""));
    Ok(())
}

#[cfg(unix)]
#[test]
fn status_reports_bad_link_target_as_drift() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    std::os::unix::fs::symlink("elsewhere", fixture.linked.join(".claude"))?;
    let context = discover_repo(&fixture.main)?;
    save_config(
        &context,
        &Config {
            version: 1,
            default_sync_policy: SyncPolicy::Manual,
            default_conflict_policy: ConflictPolicy::Ask,
            paths: vec![PathConfig {
                path: ManagedPath::parse(".claude")?,
                mode: Mode::Link,
                sync_policy: None,
                conflict_policy: None,
            }],
        },
    )?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["status", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"mode\":\"link\""))
        .stdout(predicate::str::contains("\"status\":\"drift\""));
    Ok(())
}

#[test]
fn status_clears_converged_sync_conflict_and_persists_clean_state() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "resolved")?;
    fixture.write_linked_file("docs/local/shared.md", "resolved")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &sync_config()?)?;
    let worktree = context
        .non_source_worktrees()
        .next()
        .ok_or("missing worktree")?;
    let path = ManagedPath::parse("docs/local")?;
    let state = StateStore::new(&context.control_dir);
    state.save_path_state(&PathState {
        path: path.clone(),
        worktree_id: worktree.id.clone(),
        status: PairStatus::Conflict,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::SyncCopy,
            created_or_adopted_by_wk: true,
            expected_symlink_target: None,
        },
        source_manifest: Some(empty_manifest()),
        worktree_manifest: Some(empty_manifest()),
        conflict: Some(ConflictRecord {
            entries: vec!["shared.md".into()],
            message: "manual conflict".to_owned(),
        }),
    })?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\":\"clean\""));

    let refreshed = state
        .load_path_state(&path, &worktree.id)?
        .ok_or("missing refreshed state")?;
    assert_eq!(refreshed.status, PairStatus::Clean);
    assert!(refreshed.conflict.is_none());
    Ok(())
}

const fn cli(command: Command) -> Cli {
    Cli {
        non_interactive: true,
        dry_run: false,
        command,
    }
}

fn save_config(context: &wk::git_repo::RepoContext, config: &Config) -> TestResult {
    ensure_private_dir(&context.control_dir)?;
    save_config_atomic(&context.control_dir.join("config.toml"), config)?;
    Ok(())
}

fn write_sync_config_and_clean_state(
    fixture: &GitFixture,
    source: &str,
    worktree: &str,
) -> TestResult {
    fixture.write_file("docs/local/shared.md", source)?;
    fixture.write_linked_file("docs/local/shared.md", worktree)?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &sync_config()?)?;
    let state = StateStore::new(&context.control_dir);
    let source_manifest = build_manifest(&fixture.main.join("docs/local"))?;
    let worktree_manifest = build_manifest(&fixture.linked.join("docs/local"))?;
    save_clean_state(
        &state,
        &context,
        "docs/local",
        source_manifest,
        worktree_manifest,
    )?;
    Ok(())
}

struct FixedPrompter {
    mode: Mode,
}

impl FixedPrompter {
    const fn new(mode: Mode) -> Self {
        Self { mode }
    }
}

impl Prompter for FixedPrompter {
    fn select_mode(&self, _path: &ManagedPath) -> Result<Mode, wk::error::WkError> {
        Ok(self.mode)
    }

    fn select_sync_policy(&self, _path: &ManagedPath) -> Result<SyncPolicy, wk::error::WkError> {
        Ok(SyncPolicy::Manual)
    }

    fn select_conflict_policy(
        &self,
        _path: &ManagedPath,
    ) -> Result<ConflictPolicy, wk::error::WkError> {
        Ok(ConflictPolicy::Ask)
    }

    fn confirm(&self, _message: &str, default: bool) -> Result<bool, wk::error::WkError> {
        Ok(default)
    }
}

trait ConfigTestDefaults {
    fn default_for_tests() -> Self;
}

impl ConfigTestDefaults for Config {
    fn default_for_tests() -> Self {
        Self {
            version: 1,
            default_sync_policy: SyncPolicy::Manual,
            default_conflict_policy: ConflictPolicy::Ask,
            paths: Vec::new(),
        }
    }
}

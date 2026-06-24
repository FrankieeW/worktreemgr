mod support;

use assert_cmd::Command as AssertCommand;
use filetime::{FileTime, set_file_mtime};
use predicates::prelude::*;
use support::{GitFixture, empty_manifest, save_clean_state, sync_config};
use wk::{
    atomic::ensure_private_dir,
    config::{Config, load_config, save_config_atomic},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    git_repo::{WorktreeId, discover_repo},
    lock::MutationLock,
    manifest::Manifest,
    state::{DestinationKind, MaterializationProvenance, PairStatus, PathState, StateStore},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn sync_without_args_syncs_all_sync_paths_and_worktrees() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    fixture.write_linked_file("docs/local/worktree.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &sync_config()?)?;
    let state = StateStore::new(&context.control_dir);
    save_clean_state(
        &state,
        &context,
        "docs/local",
        empty_manifest(),
        empty_manifest(),
    )?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("sync")
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/source.md"))?,
        "source"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.main.join("docs/local/worktree.md"))?,
        "worktree"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn mode_link_to_copy_round_trip_uses_binary() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &empty_config())?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["mode", "docs/local", "link"])
        .assert()
        .success();
    assert!(
        std::fs::symlink_metadata(fixture.linked.join("docs/local"))?
            .file_type()
            .is_symlink()
    );

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["mode", "docs/local", "copy"])
        .assert()
        .success();
    assert!(
        !std::fs::symlink_metadata(fixture.linked.join("docs/local"))?
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/source.md"))?,
        "source"
    );
    Ok(())
}

#[test]
fn mode_dry_run_sync_prints_plan_and_writes_nothing() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &empty_config())?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["--dry-run", "mode", "docs/local", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dry run"));
    assert!(!fixture.linked.join("docs/local/source.md").exists());
    Ok(())
}

#[test]
fn mode_to_sync_initializes_state_for_missing_destination() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &empty_config())?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["mode", "docs/local", "sync"])
        .assert()
        .success();

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\":\"clean\""));
    Ok(())
}

#[test]
fn mode_choice_source_wins_is_driven_through_binary() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "source")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &empty_config())?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["mode", "docs/local", "sync", "--choice", "source"])
        .assert()
        .success();

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/shared.md"))?,
        "source"
    );
    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\":\"clean\""));
    Ok(())
}

#[test]
fn sync_to_link_default_skip_keeps_config_in_sync_mode() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "source")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &sync_config()?)?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["mode", "docs/local", "link"])
        .assert()
        .success()
        .stderr(predicate::str::contains("skipped"));

    let config = load_config(&context.control_dir.join("config.toml"))?;
    assert_eq!(config.paths[0].mode, Mode::Sync);
    Ok(())
}

#[test]
fn mode_newer_policy_emits_unsafe_mtime_warning() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &empty_config())?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args([
            "--dry-run",
            "mode",
            "docs/local",
            "sync",
            "--conflict-policy",
            "newer",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("mtime"));
    Ok(())
}

#[test]
fn mutating_command_fails_fast_when_lock_is_held() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("AGENTS.local.md", "agents")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &empty_config())?;
    let _lock = MutationLock::acquire(&context.control_dir)?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("apply")
        .assert()
        .failure()
        .stderr(predicate::str::contains("mutation lock"));
    Ok(())
}

#[test]
fn prune_handles_git_prunable_missing_worktree() -> TestResult {
    let fixture = GitFixture::new()?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    let worktree_id = context
        .non_source_worktrees()
        .next()
        .ok_or("missing worktree")?
        .id
        .clone();
    let path = ManagedPath::parse("docs/local")?;
    state.save_path_state(&PathState {
        path: path.clone(),
        worktree_id: worktree_id.clone(),
        status: PairStatus::Clean,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::SyncCopy,
            created_or_adopted_by_wk: true,
            expected_symlink_target: None,
        },
        source_manifest: Some(Manifest::default()),
        worktree_manifest: Some(Manifest::default()),
        conflict: None,
    })?;
    std::fs::remove_dir_all(&fixture.linked)?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("prune")
        .assert()
        .success();

    assert!(state.load_path_state(&path, &worktree_id)?.is_none());
    Ok(())
}

#[test]
fn prune_removes_stale_worktree_state_but_not_backups() -> TestResult {
    let fixture = GitFixture::new()?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    let path = ManagedPath::parse("docs/local")?;
    state.save_path_state(&PathState {
        path: path.clone(),
        worktree_id: WorktreeId::linked("stale"),
        status: PairStatus::Clean,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::SyncCopy,
            created_or_adopted_by_wk: true,
            expected_symlink_target: None,
        },
        source_manifest: Some(Manifest::default()),
        worktree_manifest: Some(Manifest::default()),
        conflict: None,
    })?;
    let backup = context.control_dir.join("backups/keep.txt");
    write_file(&backup, "backup")?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("prune")
        .assert()
        .success();

    assert!(
        state
            .load_path_state(&path, &WorktreeId::linked("stale"))?
            .is_none()
    );
    assert!(backup.exists());
    Ok(())
}

#[test]
fn dry_run_gc_force_keeps_old_backups() -> TestResult {
    let fixture = GitFixture::new()?;
    let context = discover_repo(&fixture.main)?;
    let backups = context.control_dir.join("backups");
    ensure_private_dir(&backups)?;
    let old = backups.join("old.txt");
    write_file(&old, "old")?;
    set_file_mtime(&old, FileTime::from_unix_time(946_684_800, 0))?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["--dry-run", "gc", "--older-than", "30d", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("old.txt"));

    assert!(old.exists());
    Ok(())
}

#[test]
fn gc_previews_old_backups_and_honors_keep_on_force() -> TestResult {
    let fixture = GitFixture::new()?;
    let context = discover_repo(&fixture.main)?;
    let backups = context.control_dir.join("backups");
    ensure_private_dir(&backups)?;
    let old_a = backups.join("old-a.txt");
    let old_b = backups.join("old-b.txt");
    let fresh = backups.join("fresh.txt");
    write_file(&old_a, "a")?;
    write_file(&old_b, "b")?;
    write_file(&fresh, "fresh")?;
    set_file_mtime(&old_a, FileTime::from_unix_time(946_684_800, 0))?;
    set_file_mtime(&old_b, FileTime::from_unix_time(946_771_200, 0))?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["gc", "--older-than", "30d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("old-a.txt"))
        .stdout(predicate::str::contains("old-b.txt"));
    assert!(old_a.exists() && old_b.exists() && fresh.exists());

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["gc", "--older-than", "720h"])
        .assert()
        .success()
        .stdout(predicate::str::contains("old-a.txt"));

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["gc", "--older-than", "30d", "--keep", "1", "--force"])
        .assert()
        .success();
    assert!(!old_a.exists());
    assert!(old_b.exists());
    assert!(fresh.exists());
    Ok(())
}

#[test]
fn gc_removes_old_directory_backup_units() -> TestResult {
    let fixture = GitFixture::new()?;
    let context = discover_repo(&fixture.main)?;
    let backups = context.control_dir.join("backups");
    ensure_private_dir(&backups)?;
    let old_dir = backups.join("old-dir");
    write_file(&old_dir.join("nested.txt"), "old")?;
    set_file_mtime(&old_dir, FileTime::from_unix_time(946_684_800, 0))?;
    set_file_mtime(
        old_dir.join("nested.txt"),
        FileTime::from_unix_time(946_684_800, 0),
    )?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["gc", "--older-than", "30d", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("old-dir"));

    assert!(!old_dir.exists());
    Ok(())
}

fn save_config(context: &wk::git_repo::RepoContext, config: &Config) -> TestResult {
    ensure_private_dir(&context.control_dir)?;
    save_config_atomic(&context.control_dir.join("config.toml"), config)?;
    Ok(())
}

const fn empty_config() -> Config {
    Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: Vec::new(),
    }
}

fn write_file(path: &camino::Utf8Path, contents: &str) -> TestResult {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

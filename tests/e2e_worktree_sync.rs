mod e2e_support;
mod support;

use assert_cmd::Command as AssertCommand;
use e2e_support::{
    add_worktree, copy_config, empty_config, link_config, mixed_config, save_config,
};
use predicates::prelude::*;
use support::{GitFixture, empty_manifest, save_clean_state, sync_config};
use wk::{git_repo::discover_repo, manifest::build_manifest, state::StateStore};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn apply_creates_links_copies_and_sync_copies() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    fixture.write_file("AGENTS.local.md", "agents")?;
    fixture.write_file("docs/local/note.md", "note")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &mixed_config()?)?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("apply")
        .assert()
        .success();

    assert!(
        std::fs::symlink_metadata(fixture.linked.join(".claude"))?
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("AGENTS.local.md"))?,
        "agents"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/note.md"))?,
        "note"
    );
    Ok(())
}

#[test]
fn directory_sync_preserves_unique_entries_and_conflicts_per_entry() -> TestResult {
    let fixture = GitFixture::new()?;
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
    fixture.write_file("docs/local/source-only.md", "source")?;
    fixture.write_linked_file("docs/local/worktree-only.md", "worktree")?;
    fixture.write_file("docs/local/shared.md", "source edit")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree edit")?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("sync")
        .assert()
        .success()
        .stderr(predicate::str::contains("conflict"));

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/source-only.md"))?,
        "source"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.main.join("docs/local/worktree-only.md"))?,
        "worktree"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.main.join("docs/local/shared.md"))?,
        "source edit"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/shared.md"))?,
        "worktree edit"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn foreign_symlink_is_not_repaired_without_confirmation() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    let foreign_target = fixture.main.join("foreign");
    std::fs::create_dir_all(&foreign_target)?;
    std::os::unix::fs::symlink(&foreign_target, fixture.linked.join(".claude"))?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &link_config()?)?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .arg("apply")
        .assert()
        .success()
        .stderr(predicate::str::contains("foreign symlink"));

    assert_eq!(
        std::fs::read_link(fixture.linked.join(".claude"))?,
        foreign_target
    );
    Ok(())
}

#[test]
fn new_worktree_requires_manual_apply() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("AGENTS.local.md", "agents")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &copy_config()?)?;
    let second = add_worktree(&fixture, "second")?;

    assert!(!second.join("AGENTS.local.md").exists());
    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["apply", second.as_str()])
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(second.join("AGENTS.local.md"))?,
        "agents"
    );
    Ok(())
}

#[test]
fn status_json_exit_codes_are_scriptable() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "base")?;
    fixture.write_linked_file("docs/local/shared.md", "base")?;
    let context = discover_repo(&fixture.main)?;
    save_config(&context, &sync_config()?)?;
    let state = StateStore::new(&context.control_dir);
    save_clean_state(
        &state,
        &context,
        "docs/local",
        build_manifest(&fixture.main.join("docs/local"))?,
        build_manifest(&fixture.linked.join("docs/local"))?,
    )?;
    fixture.write_file("docs/local/shared.md", "source edit")?;

    AssertCommand::cargo_bin("wk")?
        .current_dir(&fixture.main)
        .args(["status", "--json"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"status\":\"drift\""));
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

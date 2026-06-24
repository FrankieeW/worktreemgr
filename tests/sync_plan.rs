mod support;

use support::{GitFixture, empty_manifest, save_clean_state, sync_config};
use wk::{
    domain::{ConflictPolicy, SyncPolicy},
    fs_plan::execute_plan,
    git_repo::discover_repo,
    manifest::build_manifest,
    state::StateStore,
    sync_plan::{SyncOptions, SyncSelector, plan_sync},
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const fn ask_manual(dry_run: bool) -> SyncOptions {
    SyncOptions {
        policy: SyncPolicy::Manual,
        conflict_policy: ConflictPolicy::Ask,
        dry_run,
    }
}

#[test]
fn sync_all_unions_source_and_worktree_added_entries() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    fixture.write_linked_file("docs/local/worktree.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    save_clean_state(
        &state,
        &context,
        "docs/local",
        empty_manifest(),
        empty_manifest(),
    )?;

    let plan = plan_sync(
        &context,
        &sync_config()?,
        &state,
        &SyncSelector::All,
        ask_manual(false),
    )?;
    execute_plan(&plan.fs_ops, false)?;

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

#[test]
fn sync_without_existing_state_uses_empty_base_for_initial_union() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source.md", "source")?;
    fixture.write_linked_file("docs/local/worktree.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_sync(
        &context,
        &sync_config()?,
        &state,
        &SyncSelector::All,
        ask_manual(false),
    )?;
    execute_plan(&plan.fs_ops, false)?;

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

#[test]
fn conflicting_entry_is_left_untouched_with_ask_policy() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "base")?;
    fixture.write_linked_file("docs/local/shared.md", "base")?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    let base_source = build_manifest(&fixture.main.join("docs/local"))?;
    let base_worktree = build_manifest(&fixture.linked.join("docs/local"))?;
    save_clean_state(&state, &context, "docs/local", base_source, base_worktree)?;
    fixture.write_file("docs/local/shared.md", "source")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree")?;

    let plan = plan_sync(
        &context,
        &sync_config()?,
        &state,
        &SyncSelector::All,
        ask_manual(false),
    )?;

    assert!(plan.fs_ops.is_empty());
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("conflict"))
    );
    Ok(())
}

#[test]
fn identical_convergence_refreshes_state_without_copying() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "base")?;
    fixture.write_linked_file("docs/local/shared.md", "base")?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    let base_source = build_manifest(&fixture.main.join("docs/local"))?;
    let base_worktree = build_manifest(&fixture.linked.join("docs/local"))?;
    save_clean_state(&state, &context, "docs/local", base_source, base_worktree)?;
    fixture.write_file("docs/local/shared.md", "same")?;
    fixture.write_linked_file("docs/local/shared.md", "same")?;

    let plan = plan_sync(
        &context,
        &sync_config()?,
        &state,
        &SyncSelector::All,
        ask_manual(false),
    )?;

    assert!(plan.fs_ops.is_empty());
    assert_eq!(plan.state_updates.len(), 1);
    Ok(())
}

#[test]
fn newer_policy_emits_unsafe_warning() -> TestResult {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "base")?;
    fixture.write_linked_file("docs/local/shared.md", "base")?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    let base_source = build_manifest(&fixture.main.join("docs/local"))?;
    let base_worktree = build_manifest(&fixture.linked.join("docs/local"))?;
    save_clean_state(&state, &context, "docs/local", base_source, base_worktree)?;
    fixture.write_file("docs/local/shared.md", "source")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree")?;

    let plan = plan_sync(
        &context,
        &sync_config()?,
        &state,
        &SyncSelector::All,
        SyncOptions {
            policy: SyncPolicy::Manual,
            conflict_policy: ConflictPolicy::Newer,
            dry_run: true,
        },
    )?;

    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("mtime"))
    );
    Ok(())
}

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::{TempDir, tempdir};
use wk::{
    config::{Config, PathConfig},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    fs_plan::{FsOp, execute_plan},
    git_repo::discover_repo,
    materialize::{ApplyTarget, OperationPlan, StateUpdate, plan_apply},
    mode_plan::{ModeOptions, TransitionChoice, plan_mode_change},
    state::{DestinationKind, MaterializationProvenance, PairStatus, PathState, StateStore},
};

#[test]
fn apply_all_excludes_source_and_links_with_relative_target()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for(".claude", Mode::Link)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_apply(&context, &config, &state, &ApplyTarget::All)?;

    assert_eq!(copy_symlink_ops(&plan).len(), 1);
    let (target, dest) = copy_symlink_ops(&plan)[0];
    assert_eq!(dest, &fixture.linked.join(".claude"));
    assert!(!dest.starts_with(&fixture.main));
    assert!(!target.is_absolute());
    Ok(())
}

#[test]
fn link_apply_records_symlink_provenance() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for(".claude", Mode::Link)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_apply(&context, &config, &state, &ApplyTarget::All)?;

    assert!(plan.state_updates.iter().any(|update| {
        matches!(
            update,
            StateUpdate::Save(path_state)
                if path_state.path.as_str() == ".claude"
                    && path_state.provenance.destination_kind == DestinationKind::Symlink
                    && path_state.provenance.created_or_adopted_by_wk
                    && path_state.provenance.expected_symlink_target.is_some()
        )
    }));
    Ok(())
}

#[cfg(unix)]
#[test]
fn link_repair_only_applies_to_wk_created_symlink() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    std::os::unix::fs::symlink("elsewhere", fixture.linked.join(".claude"))?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for(".claude", Mode::Link)?;
    let state = StateStore::new(&context.control_dir);
    state.save_path_state(&PathState {
        path: ManagedPath::parse(".claude")?,
        worktree_id: context
            .non_source_worktrees()
            .next()
            .ok_or("missing worktree")?
            .id
            .clone(),
        status: PairStatus::Clean,
        provenance: MaterializationProvenance {
            destination_kind: DestinationKind::Symlink,
            created_or_adopted_by_wk: true,
            expected_symlink_target: Some(Utf8PathBuf::from("elsewhere")),
        },
        source_manifest: None,
        worktree_manifest: None,
        conflict: None,
    })?;

    let repair = plan_apply(&context, &config, &state, &ApplyTarget::All)?;
    assert_eq!(copy_symlink_ops(&repair).len(), 1);

    let foreign_state = StateStore::new(&fixture.main.join(".foreign-wk"));
    let foreign = plan_apply(&context, &config, &foreign_state, &ApplyTarget::All)?;
    assert!(copy_symlink_ops(&foreign).is_empty());
    assert!(
        foreign
            .warnings
            .iter()
            .any(|warning| warning.contains("foreign"))
    );
    Ok(())
}

#[test]
fn mode_planner_covers_all_non_identity_transitions() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file(".claude/settings.json", "{}")?;
    let context = discover_repo(&fixture.main)?;
    let state = StateStore::new(&context.control_dir);
    let modes = [Mode::Ignore, Mode::Link, Mode::Copy, Mode::Sync];

    for from in modes {
        for to in modes {
            if from == to {
                continue;
            }
            let config = config_for(".claude", from)?;
            let plan = plan_mode_change(
                &context,
                &config,
                &state,
                &ManagedPath::parse(".claude")?,
                to,
                ModeOptions {
                    dry_run: true,
                    choice: TransitionChoice::Default,
                },
            )?;
            assert!(!plan.summary.is_empty(), "{from:?} -> {to:?}");
        }
    }
    Ok(())
}

#[test]
fn copy_to_sync_source_wins_uses_overlay_without_deleting_destination_unique_files()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/source-only.md", "source")?;
    fixture.write_linked_file("docs/local/worktree-only.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for("docs/local", Mode::Copy)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_mode_change(
        &context,
        &config,
        &state,
        &ManagedPath::parse("docs/local")?,
        Mode::Sync,
        ModeOptions {
            dry_run: false,
            choice: TransitionChoice::SourceWins,
        },
    )?;
    execute_plan(&plan.fs_ops, false)?;

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/source-only.md"))?,
        "source"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/worktree-only.md"))?,
        "worktree"
    );
    Ok(())
}

#[test]
fn ignore_to_copy_keeps_existing_destination_by_default() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = GitFixture::new()?;
    fixture.write_file("AGENTS.local.md", "source")?;
    fixture.write_linked_file("AGENTS.local.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for("AGENTS.local.md", Mode::Ignore)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_mode_change(
        &context,
        &config,
        &state,
        &ManagedPath::parse("AGENTS.local.md")?,
        Mode::Copy,
        ModeOptions {
            dry_run: false,
            choice: TransitionChoice::Default,
        },
    )?;
    execute_plan(&plan.fs_ops, false)?;

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("AGENTS.local.md"))?,
        "worktree"
    );
    Ok(())
}

#[test]
fn sync_to_copy_keeps_worktree_content_and_removes_state() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "source")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for("docs/local", Mode::Sync)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_mode_change(
        &context,
        &config,
        &state,
        &ManagedPath::parse("docs/local")?,
        Mode::Copy,
        ModeOptions {
            dry_run: false,
            choice: TransitionChoice::Default,
        },
    )?;
    execute_plan(&plan.fs_ops, false)?;

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/shared.md"))?,
        "worktree"
    );
    assert!(plan.state_updates.iter().any(|update| {
        matches!(
            update,
            StateUpdate::Remove { path, .. } if path.as_str() == "docs/local"
        )
    }));
    Ok(())
}

#[test]
fn source_wins_transition_backs_up_replaced_files() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::new()?;
    fixture.write_file("docs/local/shared.md", "source")?;
    fixture.write_linked_file("docs/local/shared.md", "worktree")?;
    let context = discover_repo(&fixture.main)?;
    let config = config_for("docs/local", Mode::Copy)?;
    let state = StateStore::new(&context.control_dir);

    let plan = plan_mode_change(
        &context,
        &config,
        &state,
        &ManagedPath::parse("docs/local")?,
        Mode::Sync,
        ModeOptions {
            dry_run: false,
            choice: TransitionChoice::SourceWins,
        },
    )?;
    let backup = plan
        .fs_ops
        .iter()
        .find_map(|op| match op {
            FsOp::BackupPath { path, backup }
                if path == &fixture.linked.join("docs/local/shared.md") =>
            {
                Some(backup.clone())
            }
            _ => None,
        })
        .ok_or("missing backup for replaced worktree file")?;
    assert!(backup.starts_with(context.control_dir.join("backups")));

    execute_plan(&plan.fs_ops, false)?;

    assert_eq!(
        std::fs::read_to_string(fixture.linked.join("docs/local/shared.md"))?,
        "source"
    );
    assert_eq!(std::fs::read_to_string(backup)?, "worktree");
    Ok(())
}

fn copy_symlink_ops(plan: &OperationPlan) -> Vec<(&Utf8PathBuf, &Utf8PathBuf)> {
    plan.fs_ops
        .iter()
        .filter_map(|op| match op {
            FsOp::CopySymlink { target, dest, .. } => Some((target, dest)),
            FsOp::CreateDir { .. }
            | FsOp::CopyFile { .. }
            | FsOp::RemoveFile { .. }
            | FsOp::RemoveEmptyDir { .. }
            | FsOp::BackupPath { .. }
            | FsOp::WriteFileAtomic { .. }
            | FsOp::SetPermissions { .. } => None,
        })
        .collect()
}

fn config_for(path: &str, mode: Mode) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: vec![PathConfig {
            path: ManagedPath::parse(path)?,
            mode,
            sync_policy: None,
            conflict_policy: None,
        }],
    })
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

    fn write_file(&self, path: &str, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        write_file(&self.main, path, contents)
    }

    fn write_linked_file(
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

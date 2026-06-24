use camino::{Utf8Path, Utf8PathBuf};

use crate::{control_store::control_dir, error::WkError};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorktreeId(String);

impl WorktreeId {
    pub fn main() -> Self {
        Self("main".to_owned())
    }

    pub fn linked(value: &str) -> Self {
        Self(value.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeInfo {
    pub id: WorktreeId,
    pub path: Utf8PathBuf,
    pub is_source: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoContext {
    pub main_worktree: Utf8PathBuf,
    pub git_common_dir: Utf8PathBuf,
    pub control_dir: Utf8PathBuf,
    pub worktrees: Vec<WorktreeInfo>,
}

impl RepoContext {
    pub fn non_source_worktrees(&self) -> impl Iterator<Item = &WorktreeInfo> {
        self.worktrees.iter().filter(|worktree| !worktree.is_source)
    }
}

pub fn discover_repo(start: &Utf8Path) -> Result<RepoContext, WkError> {
    let start = canonicalize_utf8(start)?;
    let common_raw = git_stdout(&start, &["rev-parse", "--git-common-dir"])?;
    let git_common_dir = resolve_git_path(&start, common_raw.trim())?;
    let main_worktree = main_worktree_from_common_dir(&git_common_dir)?;
    let worktrees = discover_worktrees(&main_worktree, &git_common_dir)?;
    Ok(RepoContext {
        control_dir: control_dir(&main_worktree),
        main_worktree,
        git_common_dir,
        worktrees,
    })
}

fn discover_worktrees(
    main_worktree: &Utf8Path,
    git_common_dir: &Utf8Path,
) -> Result<Vec<WorktreeInfo>, WkError> {
    let output = git_stdout(main_worktree, &["worktree", "list", "--porcelain"])?;
    let mut worktrees = output
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(|path| worktree_info(main_worktree, git_common_dir, path))
        .collect::<Result<Vec<_>, _>>()?;
    worktrees.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(worktrees)
}

fn worktree_info(
    main_worktree: &Utf8Path,
    git_common_dir: &Utf8Path,
    raw_path: &str,
) -> Result<WorktreeInfo, WkError> {
    let path = canonicalize_utf8(Utf8Path::new(raw_path))?;
    let is_source = path == main_worktree;
    let id = if is_source {
        WorktreeId::main()
    } else {
        linked_worktree_id(&path, git_common_dir)?
    };
    Ok(WorktreeInfo {
        id,
        path,
        is_source,
    })
}

fn linked_worktree_id(path: &Utf8Path, git_common_dir: &Utf8Path) -> Result<WorktreeId, WkError> {
    let git_dir_raw = git_stdout(path, &["rev-parse", "--git-dir"])?;
    let git_dir = resolve_git_path(path, git_dir_raw.trim())?;
    let worktrees_dir = git_common_dir.join("worktrees");
    if !git_dir.starts_with(&worktrees_dir) {
        return Err(WkError::unsupported_repository(format!(
            "linked worktree git dir is outside {worktrees_dir}: {git_dir}"
        )));
    }
    let id = git_dir.file_name().ok_or_else(|| {
        WkError::unsupported_repository(format!("missing worktree id: {git_dir}"))
    })?;
    Ok(WorktreeId::linked(id))
}

fn main_worktree_from_common_dir(git_common_dir: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    if git_common_dir.file_name() != Some(".git") {
        return Err(WkError::unsupported_repository(format!(
            "expected common git dir to be a normal .git directory, got {git_common_dir}"
        )));
    }
    git_common_dir
        .parent()
        .map(Utf8Path::to_path_buf)
        .ok_or_else(|| WkError::unsupported_repository("common git dir has no parent".to_owned()))
}

fn resolve_git_path(cwd: &Utf8Path, raw: &str) -> Result<Utf8PathBuf, WkError> {
    let path = Utf8PathBuf::from(raw);
    let absolute = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    canonicalize_utf8(&absolute)
}

fn canonicalize_utf8(path: &Utf8Path) -> Result<Utf8PathBuf, WkError> {
    let canonical = std::fs::canonicalize(path)?;
    Utf8PathBuf::from_path_buf(canonical)
        .map_err(|path| WkError::non_utf8_path(path.display().to_string()))
}

fn git_stdout(cwd: &Utf8Path, args: &[&str]) -> Result<String, WkError> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(WkError::git_command(cwd, args, stderr));
    }
    String::from_utf8(output.stdout).map_err(|error| WkError::message(error.to_string()))
}

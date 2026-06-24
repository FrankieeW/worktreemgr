use std::collections::BTreeMap;

use camino::Utf8Path;
use globset::{Glob, GlobSetBuilder};
use walkdir::{DirEntry, WalkDir};

use crate::{domain::ManagedPath, error::WkError, git_repo::RepoContext};

const DEFAULT_CANDIDATES: &[&str] = &[
    ".claude",
    ".codex",
    ".cursor",
    ".continue",
    "AGENTS.local.md",
    "CLAUDE.local.md",
    "GEMINI.local.md",
    "CODEX.local.md",
    "docs/local",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveryOptions {
    pub include_defaults: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredPath {
    pub path: ManagedPath,
    pub source: DiscoverySource,
    pub ignored: bool,
    pub forced: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoverySource {
    Heuristic,
    Explicit,
    Glob,
}

pub fn discover_candidates(
    ctx: &RepoContext,
    options: DiscoveryOptions,
) -> Result<Vec<DiscoveredPath>, WkError> {
    let mut discovered = BTreeMap::new();
    if options.include_defaults {
        for candidate in DEFAULT_CANDIDATES {
            let path = ManagedPath::parse(candidate)?;
            if !source_path_exists(ctx, &path) {
                continue;
            }
            if let Some(item) =
                discover_path(ctx, path, DiscoverySource::Heuristic, options.force, false)?
            {
                discovered.insert(item.path.clone(), item);
            }
        }
    }
    Ok(discovered.into_values().collect())
}

pub fn expand_explicit_or_glob(
    ctx: &RepoContext,
    input: &str,
    force: bool,
) -> Result<Vec<DiscoveredPath>, WkError> {
    if has_glob_meta(input) {
        return expand_glob(ctx, input, force);
    }
    let path = ManagedPath::parse(input)?;
    discover_path(ctx, path, DiscoverySource::Explicit, force, true)
        .map(|item| item.map_or_else(Vec::new, |discovered| vec![discovered]))
}

pub fn ensure_manageable_ignored(
    ctx: &RepoContext,
    path: &ManagedPath,
    force: bool,
) -> Result<(), WkError> {
    if is_git_ignored(ctx, path)? || force {
        return Ok(());
    }
    Err(WkError::message(format!(
        "path is not ignored by git: {path}"
    )))
}

fn expand_glob(
    ctx: &RepoContext,
    input: &str,
    force: bool,
) -> Result<Vec<DiscoveredPath>, WkError> {
    let matcher = glob_matcher(input)?;
    let mut discovered = BTreeMap::new();
    for entry in WalkDir::new(&ctx.main_worktree)
        .follow_links(false)
        .into_iter()
        .filter_entry(should_descend)
    {
        let entry = entry?;
        let relative = relative_entry_path(&ctx.main_worktree, &entry)?;
        if relative.as_str().is_empty() || is_reserved(relative) {
            continue;
        }
        if !glob_matches(input, &matcher, relative) {
            continue;
        }
        let path = ManagedPath::parse(relative.as_str())?;
        if let Some(item) = discover_path(ctx, path, DiscoverySource::Glob, force, false)? {
            discovered.insert(item.path.clone(), item);
        }
    }
    Ok(discovered.into_values().collect())
}

fn discover_path(
    ctx: &RepoContext,
    path: ManagedPath,
    source: DiscoverySource,
    force: bool,
    reject_unignored: bool,
) -> Result<Option<DiscoveredPath>, WkError> {
    let ignored = is_git_ignored(ctx, &path)?;
    if !ignored && !force {
        if reject_unignored {
            return Err(WkError::message(format!(
                "path is not ignored by git: {path}"
            )));
        }
        return Ok(None);
    }
    Ok(Some(DiscoveredPath {
        path,
        source,
        ignored,
        forced: force && !ignored,
    }))
}

fn is_git_ignored(ctx: &RepoContext, path: &ManagedPath) -> Result<bool, WkError> {
    let status = std::process::Command::new("git")
        .current_dir(&ctx.main_worktree)
        .args(["check-ignore", "-q", "--", path.as_str()])
        .status()?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => Err(WkError::message(format!(
            "git check-ignore failed with exit code {code} for {path}"
        ))),
        None => Err(WkError::message(format!(
            "git check-ignore terminated by signal for {path}"
        ))),
    }
}

fn source_path_exists(ctx: &RepoContext, path: &ManagedPath) -> bool {
    ctx.main_worktree.join(path.as_path()).exists()
}

fn glob_matcher(input: &str) -> Result<globset::GlobSet, WkError> {
    let glob = Glob::new(input).map_err(|error| WkError::message(error.to_string()))?;
    let mut builder = GlobSetBuilder::new();
    builder.add(glob);
    builder
        .build()
        .map_err(|error| WkError::message(error.to_string()))
}

fn glob_matches(input: &str, matcher: &globset::GlobSet, relative: &Utf8Path) -> bool {
    if input.contains('/') {
        return matcher.is_match(relative.as_str());
    }
    relative
        .file_name()
        .is_some_and(|file_name| matcher.is_match(file_name))
}

fn should_descend(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    name != ".git" && name != ".wk"
}

fn relative_entry_path<'a>(root: &Utf8Path, entry: &'a DirEntry) -> Result<&'a Utf8Path, WkError> {
    let path = Utf8Path::from_path(entry.path())
        .ok_or_else(|| WkError::non_utf8_path(entry.path().display().to_string()))?;
    path.strip_prefix(root)
        .map_err(|error| WkError::message(error.to_string()))
}

fn is_reserved(path: &Utf8Path) -> bool {
    path.as_str()
        .split('/')
        .any(|segment| segment == ".git" || segment == ".wk")
}

fn has_glob_meta(input: &str) -> bool {
    input
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
}

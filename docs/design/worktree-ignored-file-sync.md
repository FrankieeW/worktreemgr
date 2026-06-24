# Worktree Ignored File Sync Design

## Goal

Build a Rust CLI named `wk` that manages gitignored AI/local files across git
worktrees. The tool lets a user decide which ignored files are not managed,
symlinked, copied once, or kept in bidirectional sync.

The primary pain point is that useful AI/local files do not appear in new
worktrees because they are intentionally gitignored. `wk` should make those
files available without forcing them into git history.

## Scope

This design covers:

- `wk init`: interactive setup for the current repository.
- `wk apply`: materialize configured files into a worktree.
- `wk status`: show managed file state and drift.
- `wk sync`: synchronize bidirectional entries.
- `wk mode`: switch a managed path between `ignore`, `link`, `copy`, and `sync`.
- Manual or automatic sync policy for `sync` entries, with manual as the default.

This design does not cover:

- Background daemons.
- Git hook installation by default.
- Cross-machine sync.
- Storing secrets in tracked files.

## Concepts

### Source Root

Each managed path has one source copy under the main repository worktree. The
source copy is the canonical path used by `link` and the default peer used by
`copy` and `sync`.

`wk init` detects the source root from the current git repository and records
it in config.

### Managed Path

A managed path is a file or directory that is normally ignored by git and useful
inside one or more worktrees. Examples:

- `.claude/`
- `.codex/`
- `.cursor/`
- `.continue/`
- `AGENTS.local.md`
- `CLAUDE.local.md`
- `docs/local/`
- `*.local.*`

Discovery is heuristic. The user can add or remove entries during `wk init`.

### Modes

`ignore`
: `wk` records that the path exists but does not manage it.

`link`
: each target worktree gets a symlink pointing back to the source copy.

`copy`
: each target worktree gets an independent copy. Later changes are intentionally
  local to that worktree.

`sync`
: each target worktree gets a copy that can be synchronized with the source copy.
  This supports either manual sync or automatic sync.

## Configuration

The repository stores configuration in `.wk/config.toml`. The `.wk/` directory is
intended to be gitignored by default.

Sketch:

```toml
version = 1
source_root = "/Users/example/project"
default_sync_policy = "manual"

[[paths]]
path = ".claude"
mode = "link"

[[paths]]
path = "docs/local"
mode = "sync"
sync_policy = "manual"
conflict_policy = "ask"

[[paths]]
path = "AGENTS.local.md"
mode = "copy"
```

`sync_policy` values:

- `manual`: only sync when `wk sync` is run.
- `auto`: sync during `wk apply`, and during mode transitions that need content
  reconciliation.

Manual is the default because AI/local files often contain intentional
worktree-specific context.

`conflict_policy` values:

- `ask`: stop and ask the user to choose.
- `source`: source copy wins.
- `worktree`: worktree copy wins.
- `newer`: newer modified time wins.

`ask` is the default for manual sync. `auto` may use `source`, `worktree`, or
`newer`; `auto` with `ask` is allowed but becomes an interactive auto-sync.

## Commands

### `wk init`

`wk init` is interactive:

1. Detect the git repository and all existing git worktrees.
2. Scan for ignored AI/local files in the source worktree.
3. Present each candidate path with a mode selector:
   - `ignore`
   - `link`
   - `copy`
   - `sync`
4. For each `sync` path, ask for sync policy:
   - `manual` default
   - `auto`
5. For each `sync` path, ask for conflict policy when needed:
   - `ask` default
   - `source`
   - `worktree`
   - `newer`
6. Write `.wk/config.toml`.
7. Optionally run `wk apply` for currently known worktrees.

### `wk apply [worktree]`

Materialize configured paths into one target worktree or all known worktrees.

Behavior by mode:

- `ignore`: no-op.
- `link`: create or repair symlink.
- `copy`: copy only when the destination path does not exist.
- `sync`: copy when missing; if `sync_policy = "auto"`, run sync for that path.

`wk apply` must not overwrite an existing non-managed destination without asking.

### `wk status`

Show one row per managed path per worktree:

- mode
- whether destination exists
- symlink target correctness for `link`
- copy drift for `copy`
- sync drift or conflict for `sync`

For `sync`, status compares a stored content fingerprint with current source and
worktree fingerprints:

- source changed only
- worktree changed only
- both changed
- clean

### `wk sync [path] [worktree]`

Synchronize `sync` paths.

Manual policy:

- If only source changed, copy source to worktree.
- If only worktree changed, copy worktree to source.
- If both changed, use `conflict_policy`.
- With `ask`, prompt the user and do not overwrite silently.

Auto policy:

- Same algorithm, but it may run from `wk apply` or mode transitions.
- If the policy cannot decide safely, the command stops with a clear conflict.

### `wk mode <path> <mode>`

Switch one managed path between modes. This command is required because mode
changes are not just config edits; they may need filesystem migration.

All mode changes first build a migration plan, show risky actions to the user,
create required backups, execute filesystem changes, and only then persist the
updated `.wk` config and state. A failed migration must leave the previous mode
metadata intact.

Changing into `sync` accepts the same sync settings as `wk init`:

- `--sync-policy manual|auto`
- `--conflict-policy ask|source|worktree|newer`

Without flags, `manual` and `ask` are used.

## Mode Transition Rules

### Any Mode -> `ignore`

`wk` stops managing the path.

Prompt for cleanup behavior:

- keep current files in each worktree
- remove `wk`-created symlinks only
- remove `wk` metadata and state only

Default: keep files and remove only metadata/state. This avoids data loss.

### `ignore` -> `link`

Use the source copy as canonical. For each target worktree:

- If destination is missing, create symlink.
- If destination is already the correct symlink, no-op.
- If destination exists as a regular file or directory, prompt:
  - replace with symlink after backing up destination
  - skip this worktree
  - import destination into source, then link

Default: skip existing destinations.

### `ignore` -> `copy`

Copy source into each target worktree only when missing.

If destination exists, prompt:

- keep existing destination
- replace from source after backup
- skip this worktree

Default: keep existing destination.

### `ignore` -> `sync`

Initialize a sync pair for each target worktree.

If destination is missing, copy source and record fingerprints.

If destination exists, prompt:

- use source as canonical and overwrite destination after backup
- use worktree destination as canonical and copy it back to source after backup
- mark as conflict and leave content untouched

Default: mark as conflict and leave content untouched.

### `link` -> `copy`

Replace the symlink with an independent copy of the source content.

If the symlink is correct:

- remove symlink
- copy source content to destination

If the destination is not the expected symlink, treat it like an existing
destination and prompt before changing it.

### `link` -> `sync`

Replace the symlink with a copy of the source content and record clean sync
fingerprints. This starts as "source and worktree are identical".

If the destination is not the expected symlink, prompt before changing it.

### `copy` -> `link`

This discards worktree-specific copy behavior.

Prompt:

- replace destination with symlink after backup
- import destination into source, then replace with symlink
- skip this worktree

Default: backup and replace only after explicit confirmation.

### `copy` -> `sync`

Start tracking bidirectional drift.

If source and destination content are identical, record clean fingerprints.

If they differ, prompt:

- source wins
- worktree wins
- mark as conflict and do not overwrite

Default: mark as conflict.

### `sync` -> `copy`

Stop bidirectional sync but keep each worktree's current file content.

If there is an unresolved sync conflict, leave content untouched and mark the
path as `copy`.

### `sync` -> `link`

This can discard worktree-specific synced content, so it is conservative.

Before replacing with symlink:

- If clean, replace destination with symlink.
- If only source changed, apply source then replace with symlink.
- If only worktree changed or both changed, prompt:
  - sync worktree back to source, then link
  - backup worktree content, then link to source
  - skip this worktree

Default: skip when worktree content would be lost.

## State Tracking

`wk` stores state under `.wk/state/`. State is not intended for git.

For each managed sync path and worktree, record:

- source fingerprint at last clean sync
- worktree fingerprint at last clean sync
- source modified time at last clean sync
- worktree modified time at last clean sync

Fingerprints should be content hashes. Directories are fingerprinted from a
stable traversal of relative paths, file types, executable bits, symlink targets,
and file contents.

## Safety Rules

- Never overwrite an existing destination unless the selected mode transition
  explicitly allows it.
- Before replacing or deleting content, create a timestamped backup under
  `.wk/backups/`.
- Never follow symlinks outside expected managed paths when recursively copying
  directories.
- Never manage files unless they are ignored by git or explicitly forced by the
  user.
- Never sync `.git/`.
- Normalize all paths to repository-relative paths in config.

## Testing Strategy

Unit tests:

- config parse and serialization
- path discovery filtering
- directory fingerprinting
- sync drift classification
- mode transition planning

Integration tests:

- create a temporary git repository with multiple worktrees
- run `wk init` with scripted input
- run `wk apply`, `wk mode`, `wk sync`, and `wk status`
- verify filesystem results

CLI end-to-end tests:

- `init -> apply` creates symlinks, copies, and sync copies
- `copy -> sync` detects divergent content and marks conflict
- `sync manual` requires explicit command
- `sync auto` reconciles according to configured policy
- `sync -> link` refuses to discard worktree-only content without confirmation

## Implementation Notes

Preferred Rust libraries:

- `clap` for CLI parsing
- `inquire` or `dialoguer` for interactive prompts
- `serde` and `toml` for config
- `thiserror` for typed errors
- `camino` for UTF-8 paths
- `sha2` or `blake3` for content fingerprints
- `assert_cmd` and `tempfile` for CLI integration tests

The implementation should keep filesystem planning separate from filesystem
mutation. Mode transitions should first produce a typed plan, then execute it.
This makes risky transitions testable before any file is changed.

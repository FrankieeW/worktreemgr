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
- `wk add`: add a new managed concrete path after init.
- `wk apply`: materialize configured files into a worktree.
- `wk status`: show managed file state and drift.
- `wk sync`: synchronize bidirectional entries.
- `wk mode`: switch a managed path between `ignore`, `link`, `copy`, and `sync`.
- `wk prune`: remove state for deleted worktrees.
- `wk gc`: remove old backups after explicit user action.
- Manual or automatic sync policy for `sync` entries, with manual as the default.

This design does not cover:

- Background daemons.
- Git hook installation by default.
- Cross-machine sync.
- Storing secrets in tracked files.
- Bare repositories or repositories without a normal main worktree.

V1 requires a manual `wk apply` after `git worktree add`. An opt-in hook helper
can be added later, but the default behavior must stay explicit.

## Repository Layout

`wk` stores repository-local control data once, in the main worktree:

```text
<main-worktree>/.wk/config.toml
<main-worktree>/.wk/state/
<main-worktree>/.wk/backups/
<main-worktree>/.wk/lock
```

The main worktree is derived at runtime:

1. Run `git rev-parse --git-common-dir` from the current worktree.
2. Resolve the common git directory.
3. For v1, require that the common git directory is the `.git` directory inside
   a normal main worktree.
4. Use the parent directory of that `.git` directory as `<main-worktree>`.

This means `wk` can be run from any linked worktree while still reading and
writing exactly one config/state store. `wk` must not duplicate `.wk/` into each
worktree.

`wk init` must idempotently append `.wk/` to the repository `.gitignore` if it is
not already ignored. `.wk/backups/` can contain secret-bearing local files, so it
must never be tracked.

## Concepts

### Source Root

The source root is the main worktree derived from git metadata at runtime. It is
not stored as an absolute path in config.

Each managed path has one source copy under the source root. The source copy is
the canonical path used by `link` and the shared hub used by `copy` and `sync`.

### Managed Path

A managed path in config is always a concrete repository-relative path, never a
glob. Examples:

- `.claude`
- `.codex`
- `.cursor`
- `.continue`
- `AGENTS.local.md`
- `CLAUDE.local.md`
- `docs/local`

Globs such as `*.local.*` are discovery heuristics only. `wk init` and `wk add`
may use globs to find candidate files, but persisted config entries are expanded
to concrete paths. New files that later match a heuristic glob are not managed
until the user runs `wk add` or reruns discovery.

Nested `.git` and `.wk` directories are never managed.

### Worktree Identity

`wk` discovers live worktrees from `git worktree list --porcelain` on every
command. State is keyed by a stable worktree identity:

- `main` for the source worktree.
- the linked worktree git-dir name under `<common-git-dir>/worktrees/` for linked
  worktrees.

The current filesystem path of a worktree is treated as runtime data, not as the
state identity. Deleted worktree state is left alone until `wk prune`.

### Modes

`ignore`
: `wk` records that the path is known but does not manage it. This is a
  `wk` tombstone/unmanaged mode, not a `.gitignore` rule.

`link`
: each target worktree gets a symlink pointing back to the source copy.

`copy`
: each target worktree gets an independent copy. Later changes are intentionally
  local to that worktree.

`sync`
: each target worktree gets a copy that can be synchronized with the source copy.
  This supports either manual sync or automatic sync.

## Configuration

The repository stores configuration in `<main-worktree>/.wk/config.toml`.

Sketch:

```toml
version = 1
default_sync_policy = "manual"
default_conflict_policy = "ask"

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

Config paths are repository-relative concrete paths. Absolute paths and glob
patterns are rejected.

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

`ask` is the default. `newer` is best-effort and unsafe for conflict resolution
because mtimes can be rewritten by copy tools, checkout operations, and clock
skew. `newer` must never be selected as an automatic default; it is only allowed
when the user explicitly configures it.

## Commands

### `wk init`

`wk init` is interactive:

1. Detect the current git repository and derive `<main-worktree>`.
2. Reject unsupported bare/no-main-worktree layouts.
3. Create `<main-worktree>/.wk/`.
4. Idempotently add `.wk/` to `.gitignore`.
5. Discover live worktrees with `git worktree list --porcelain`.
6. Scan for ignored AI/local files in the source worktree.
7. Present each candidate concrete path with a mode selector:
   - `ignore`
   - `link`
   - `copy`
   - `sync`
8. For each `sync` path, ask for sync policy:
   - `manual` default
   - `auto`
9. For each `sync` path, ask for conflict policy when needed:
   - `ask` default
   - `source`
   - `worktree`
   - `newer`
10. Write `.wk/config.toml`.
11. Optionally run `wk apply` for currently known worktrees.

### `wk add <path>`

Add one or more new concrete paths after init.

`wk add` accepts:

- a concrete repository-relative path
- a glob used only for discovery, expanded interactively into concrete paths

For each added concrete path, `wk add` prompts for the same mode and sync
settings as `wk init`, then runs the same migration rules as
`ignore -> <mode>`.

### `wk apply [worktree]`

Materialize configured paths into one target worktree or all live worktrees.

Behavior by mode:

- `ignore`: no-op.
- `link`: create or repair symlink.
- `copy`: copy only when the destination path does not exist.
- `sync`: copy when missing; if `sync_policy = "auto"`, run sync for that path.

`wk apply` must not overwrite an existing non-managed destination without asking.

### `wk status`

Show one row per managed path per live worktree:

- mode
- whether destination exists
- symlink target correctness for `link`
- copy drift for `copy`
- sync drift or conflict for `sync`

For `sync`, status compares the last clean fingerprints with current source and
worktree fingerprints:

- clean
- source changed only
- worktree changed only
- both changed
- conflict
- uninitialized

If both sides changed but now have identical content, status is `clean` and the
last clean fingerprints are refreshed.

### `wk sync [path] [worktree]`

Synchronize `sync` paths through the source root as a hub.

Manual policy:

- If only source changed, copy source to worktree.
- If only worktree changed, copy worktree to source.
- If both changed to identical content, mark clean.
- If both changed differently, use `conflict_policy`.
- With `ask`, prompt the user and do not overwrite silently.

Auto policy:

- Same algorithm, but it may run from `wk apply` or mode transitions.
- If the policy cannot decide safely, the command stops with a clear conflict.
- `newer` is never used unless explicitly configured for that path.

Sync propagation is transitive through the source root. If worktree A syncs a
change back to source, worktree B will later observe "source changed only" and
pull that change, unless B also has local edits. If B has local edits, B becomes
a conflict even though the remote change originated in A. This is expected and
must be visible in `wk status`.

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

### `wk prune`

Discover live worktrees and remove state entries for worktrees that no longer
exist. `wk prune` must not remove backups.

### `wk gc`

Remove old backups only after explicit user action. The default command shows
what would be removed; destructive cleanup requires confirmation or a force flag.

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

Symlink targets are relative paths from the destination parent to the source
path. Relative links keep sibling worktrees movable as a group. A directory
`link` is shared state: files created under the linked directory from any
worktree are created in the source copy and become visible to all worktrees.

### `ignore` -> `copy`

Copy source into each target worktree only when missing.

If destination exists, prompt:

- keep existing destination
- replace from source after backup
- skip this worktree

Default: keep existing destination.

### `ignore` -> `sync`

Initialize a sync pair for each target worktree.

If destination is missing, copy source and record clean fingerprints.

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

`wk` stores state under `<main-worktree>/.wk/state/`. State is not intended for
git.

For each managed sync path and worktree, record:

- path
- worktree identity
- pair status: `uninitialized`, `clean`, or `conflict`
- source fingerprint at last clean sync
- worktree fingerprint at last clean sync
- source modified time and size at last clean sync
- worktree modified time and size at last clean sync
- conflict details when status is `conflict`

A conflict is cleared only when the user selects a resolution through `wk sync`
or `wk mode`, or when both sides converge to identical content and `wk status`
refreshes the clean fingerprints.

Fingerprints are content hashes. Directories are fingerprinted from a stable
traversal of relative paths, file types, executable bits, symlink targets, and
file contents. Empty directories contribute an explicit marker. Nested `.git`
and `.wk` directories are excluded from traversal.

For performance, mtime and size are a cheap pre-check. If both are unchanged
from the last clean state, `wk` may skip rehashing. If either differs, `wk` must
rehash before deciding drift or conflict.

## Safety Rules

- Never overwrite an existing destination unless the selected mode transition
  explicitly allows it.
- Before replacing or deleting content, create a timestamped backup under
  `.wk/backups/`.
- Never follow symlinks outside expected managed paths when recursively copying
  directories.
- Never manage files unless they are ignored by git or explicitly forced by the
  user.
- Never sync `.git/` or `.wk/`.
- Normalize all persisted managed paths to repository-relative concrete paths.
- Hold `<main-worktree>/.wk/lock` while mutating config, state, backups, or
  managed filesystem entries. Concurrent `wk` runs must fail fast with a clear
  lock message.

## Testing Strategy

Unit tests:

- config parse and serialization
- common-dir based control-dir resolution
- path discovery filtering and glob expansion into concrete entries
- directory fingerprinting, including empty directories and nested `.git`/`.wk`
  exclusions
- sync drift classification, including both-changed-identical convergence
- mode transition planning
- conflict status persistence and clearing

Integration tests:

- create a temporary git repository with multiple worktrees
- run `wk init` with scripted input
- assert `.wk/` is appended idempotently to `.gitignore`
- run `wk add`, `wk apply`, `wk mode`, `wk sync`, `wk status`, `wk prune`, and
  `wk gc`
- verify filesystem results
- delete a worktree and verify stale state is pruned only by `wk prune`

CLI end-to-end tests:

- `init -> apply` creates symlinks, copies, and sync copies
- `copy -> sync` detects divergent content and marks conflict
- `sync manual` requires explicit command
- `sync auto` reconciles according to configured policy
- `sync` fan-out from worktree A to source is visible to worktree B
- `sync -> link` refuses to discard worktree-only content without confirmation
- a new git worktree has no managed files until `wk apply` is run

## Implementation Notes

Preferred Rust libraries:

- `clap` for CLI parsing
- `inquire` or `dialoguer` for interactive prompts
- `serde` and `toml` for config
- `thiserror` for typed errors
- `camino` for UTF-8 paths
- `sha2` or `blake3` for content fingerprints
- `fs4` or an equivalent lockfile crate for `.wk/lock`
- `assert_cmd` and `tempfile` for CLI integration tests

The implementation should keep filesystem planning separate from filesystem
mutation. Mode transitions should first produce a typed plan, then execute it.
This makes risky transitions testable before any file is changed.

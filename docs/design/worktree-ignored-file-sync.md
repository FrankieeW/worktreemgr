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

### Copy Semantics

Directory copies are overlay copies, not mirrors:

- create missing directories and files
- replace files only when the active operation explicitly owns that path
- preserve symlinks as symlink entries instead of dereferencing them
- leave destination-only files in place
- never delete destination extras during `copy`, `apply`, or mode transitions

Mirror semantics are not part of v1. Deletions happen only in `sync`, and only
when a per-entry sync classification says that a tracked entry was deleted on
one side while unchanged on the other side.

When copying a directory that contains a symlink, `wk` copies the symlink itself.
It does not follow or dereference the symlink. Absolute symlinks and symlinks
that point outside the managed path are preserved but reported as warnings
because they may dangle or point back into the source worktree.

Transition-time reconciliation uses the same overlay copy semantics. A transition
must not make a directory identical to one side by deleting destination-only
entries. Establishing the first clean sync manifest is a separate state step
after the chosen overlay/import operation has completed.

### Worktree Identity

`wk` discovers live worktrees from `git worktree list --porcelain` on every
command. State is keyed by a stable worktree identity:

- `main` for the source worktree.
- the linked worktree git-dir name under `<common-git-dir>/worktrees/` for linked
  worktrees.

The current filesystem path of a worktree is treated as runtime data, not as the
state identity. Deleted worktree state is left alone until `wk prune`.
`main` is a source-side reference identity only. Per-destination materialization
state is recorded only for non-source worktrees.

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

### Directory Sync Semantics

`sync` supports files and directories. Directory sync is per-entry, never
whole-directory replacement.

For a managed directory, state stores a manifest of every tracked relative entry
under that directory. Each file, symlink, and empty directory is classified
independently against the last clean manifest:

- unchanged
- added on source
- added on worktree
- modified on source
- modified on worktree
- deleted on source
- deleted on worktree
- changed identically on both sides
- changed differently on both sides

Sync applies the safe per-entry plan:

- source-only additions are copied to the worktree
- worktree-only additions are copied to the source
- one-sided modifications replace only that entry on the other side
- one-sided deletions delete only that tracked entry on the other side
- identical convergent changes refresh state without copying
- conflicting entries remain untouched until resolved

A directory-level sync must never resolve a conflict by replacing the whole
directory. A resolution applies only to the conflicting entries selected by the
user.

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
when the user explicitly configures it. Interactive prompts must show this
unsafe-mtime warning when the user selects `newer`.

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

If the user selects `newer`, the picker must show the unsafe-mtime warning from
the configuration section before accepting the choice.

If config already exists, `wk init` is an idempotent merge:

- preserve existing managed paths and their modes by default
- prompt only for new discovered candidates
- show missing source paths but do not delete their config entries automatically
- update `.gitignore` idempotently
- require `--reset` to re-prompt every existing managed path

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
Without an explicit worktree argument, `wk apply` targets all live non-source
worktrees. The source/main worktree is never an apply target: its managed paths
are the source copies, not destinations.

Behavior by mode:

- `ignore`: no-op.
- `link`: create symlink, or repair only a symlink that state records as
  `wk`-created.
- `copy`: copy only when the destination path does not exist.
- `sync`: copy when missing; if `sync_policy = "auto"`, run sync for that path.

`wk apply` must not overwrite an existing non-managed destination without asking.
Foreign symlinks are existing non-managed destinations unless the user adopts
them or replaces them interactively.

`wk apply --dry-run` prints the planned filesystem operations and writes nothing.

### `wk status`

Show one row per managed path per live worktree:

- mode
- whether destination exists
- symlink target correctness for `link`
- copy drift for `copy`
- sync drift or conflict for `sync`

For `sync`, status compares the last clean manifest with current source and
worktree manifests:

- clean
- source changed only
- worktree changed only
- both changed
- conflict
- uninitialized

If both sides changed but now have identical content, status is `clean` and the
last clean manifest is refreshed.

`wk status --json` emits machine-readable status. Exit codes:

- `0`: clean
- `1`: drift exists but no conflicts
- `2`: at least one conflict exists
- `3+`: command or repository error

### `wk sync [path] [worktree]`

Synchronize `sync` paths through the source root as a hub.
With no path or worktree arguments, `wk sync` means all sync paths in all live
non-source worktrees.

Manual policy:

- If only source-side entries changed, apply those entry changes to the worktree.
- If only worktree-side entries changed, apply those entry changes to the source.
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

`wk sync --dry-run` prints the per-entry plan and writes nothing.

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
If `--conflict-policy newer` is passed, `wk` must print the unsafe-mtime warning
before building the migration plan.

`wk mode --dry-run <path> <mode>` prints the migration plan and writes nothing.

### `wk prune`

Discover live worktrees and remove state entries for worktrees that no longer
exist. `wk prune` must not remove backups.

### `wk gc`

Remove old backups only after explicit user action. By default, `wk gc` previews
backups older than 30 days. Destructive cleanup requires confirmation or
`--force`. Users can override retention with `--older-than <duration>` and
`--keep <count>`.

## Mode Transition Rules

### Any Mode -> `ignore`

`wk` stops managing the path.

Prompt for cleanup behavior. For `copy` and `sync` destinations:

- keep current files in each worktree
- remove `wk` metadata and state only

Default: keep files and remove only metadata/state. This avoids data loss.

For `link` destinations, use link-specific wording:

- convert the `wk`-created symlink to a standalone overlay copy, then unmanage it
- remove the `wk`-created symlink
- keep the live symlink unmanaged

Default: convert the symlink to a standalone copy. Keeping a live symlink
unmanaged is allowed only after an explicit warning because it continues sharing
source content invisibly.

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

When `wk` creates or adopts a link destination, it records symlink provenance in
state. Future automatic repair is allowed only for a destination recorded as
`wk`-created. A foreign symlink pointing anywhere else is treated like an
existing user destination and must not be rewritten without confirmation.

### `ignore` -> `copy`

Overlay-copy source entries into each target worktree only when missing.

If destination exists, prompt:

- keep existing destination
- overlay source entries after backup
- skip this worktree

Default: keep existing destination.

### `ignore` -> `sync`

Initialize a sync pair for each target worktree.

If destination is missing, overlay-copy source entries and record a clean
manifest.

If destination exists, prompt:

- use source as canonical and overlay source entries onto the destination after
  backup
- use worktree destination as canonical and overlay destination entries back to
  source after backup
- mark as conflict and leave content untouched

Default: mark as conflict and leave content untouched.

### `link` -> `copy`

Replace the symlink with an independent overlay copy of the source content.

If the symlink is correct:

- remove symlink
- overlay-copy source content to destination

If the destination is not the expected symlink, treat it like an existing
destination and prompt before changing it.

### `link` -> `sync`

Replace the symlink with a copy of the source content and record clean sync
manifests. This starts as "source and worktree are identical".

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

If source and destination content are identical, record a clean manifest.

If they differ, prompt:

- use source as canonical by overlaying source entries onto the worktree after
  backup
- use worktree as canonical by overlaying worktree entries back to source after
  backup
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
- If only source-side entries changed, apply those entries then replace with
  symlink.
- If only worktree changed or both changed, prompt:
  - apply worktree-side entries back to source, then link
  - backup worktree content, then link to source
  - skip this worktree

Default: skip when worktree content would be lost.

## State Tracking

`wk` stores state under `<main-worktree>/.wk/state/`. State is not intended for
git.

For each managed path and non-source worktree, record:

- path
- worktree identity
- pair status: `uninitialized`, `clean`, or `conflict`
- materialization provenance:
  - destination kind: `missing`, `copy`, `sync-copy`, `symlink`, or `foreign`
  - whether `wk` created or adopted the destination
  - expected symlink target for `wk`-created links
- source manifest at last clean sync
- worktree manifest at last clean sync
- conflict details when status is `conflict`

A conflict is cleared only when the user selects a resolution through `wk sync`
or `wk mode`, or when all conflicting entries converge to identical content and
`wk status` refreshes the clean manifest.

Manifest entries contain content hashes, file type, executable bit, symlink
target, size, and mtime. Directory manifests are built from a stable traversal of
relative paths, file types, executable bits, symlink targets, and file contents.
Empty directories contribute an explicit marker. Nested `.git` and `.wk`
directories are excluded from traversal.

For performance, mtime and size are a cheap pre-check. If both are unchanged
from the last clean state, `wk` may skip rehashing. If either differs, `wk` must
rehash before deciding drift or conflict.

For managed directories, `wk` must still enumerate the directory tree to detect
added or deleted entries. The mtime/size fast path can skip content rehashing for
unchanged entries after enumeration; it must not skip enumeration of a managed
directory.

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
- `wk status` is read-only and must not take the exclusive mutation lock. It
  reads atomic state snapshots; if a state file changes while being read, it may
  retry once before reporting a transient concurrent-update error.
- Write config and state atomically with temp-file-plus-rename so a crash cannot
  leave partial TOML or partial state files behind.
- Create `.wk/`, `.wk/state/`, and `.wk/backups/` with owner-only directory
  permissions on Unix (`0700`). Write config, state, and backup files with
  owner-only file permissions on Unix (`0600`). On non-Unix platforms, use the
  closest available restrictive permissions.

## Testing Strategy

Unit tests:

- config parse and serialization
- common-dir based control-dir resolution
- path discovery filtering and glob expansion into concrete entries
- directory manifest hashing, including empty directories and nested `.git`/`.wk`
  exclusions
- overlay copy planning for directories with destination-only files
- per-entry directory sync planning for additions, modifications, deletions, and
  conflicts
- directory manifest enumeration even when directory mtime/size appears unchanged
- sync drift classification, including both-changed-identical convergence
- mode transition planning
- conflict status persistence and clearing
- symlink provenance decisions for `wk`-created and foreign symlinks

Integration tests:

- create a temporary git repository with multiple worktrees
- run `wk init` with scripted input
- assert `.wk/` is appended idempotently to `.gitignore`
- run `wk add`, `wk apply`, `wk mode`, `wk sync`, `wk status`, `wk prune`, and
  `wk gc`
- verify filesystem results
- delete a worktree and verify stale state is pruned only by `wk prune`
- verify `wk init` rerun preserves existing config and prompts only for new
  candidates
- verify source/main worktree is not an `apply` target
- verify config/state writes are atomic under injected write failures
- verify `.wk` directories/files use restrictive permissions where the platform
  supports them
- verify `wk status` can read a stable snapshot while another command holds the
  mutation lock
- verify `wk gc --older-than` and `--keep` retention behavior

CLI end-to-end tests:

- `init -> apply` creates symlinks, copies, and sync copies
- `copy -> sync` detects divergent content and marks conflict
- `sync manual` requires explicit command
- `sync auto` reconciles according to configured policy
- `sync` fan-out from worktree A to source is visible to worktree B
- `sync -> link` refuses to discard worktree-only content without confirmation
- a new git worktree has no managed files until `wk apply` is run
- directory `sync` preserves unique files on both sides and resolves conflicts
  per entry
- `link -> ignore` converts a `wk` symlink to a standalone copy by default
- `wk apply` refuses to repair a foreign symlink without confirmation
- `wk status --json` is parseable and exit codes distinguish clean, drift, and
  conflict
- `--dry-run` on `apply`, `sync`, and `mode` writes nothing
- `wk sync` with no arguments targets all sync paths in all non-source worktrees

## Implementation Notes

Preferred Rust libraries:

- `clap` for CLI parsing
- `inquire` or `dialoguer` for interactive prompts
- `serde` and `toml` for config
- `thiserror` for typed errors
- `camino` for UTF-8 paths
- `sha2` or `blake3` for content hashes
- `fs4` or an equivalent lockfile crate for `.wk/lock`
- `assert_cmd` and `tempfile` for CLI integration tests

The implementation should keep filesystem planning separate from filesystem
mutation. Mode transitions should first produce a typed plan, then execute it.
This makes risky transitions testable before any file is changed.

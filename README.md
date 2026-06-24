# worktreemgr

`wk` manages gitignored AI and local files across Git worktrees.

It keeps repository-local control data in the main worktree under `.wk/`, then
materializes selected ignored files into linked worktrees as symlinks, one-time
copies, or bidirectional sync copies.

## Build

```bash
cargo build
cargo run -- --help
```

## Quick Start

```bash
# From any worktree in a normal non-bare repository
wk init

# Add one ignored path or a discovery glob
wk add 'AGENTS.local.md'
wk add '*.local.*'

# Materialize configured paths into all linked worktrees
wk apply

# Preview risky changes without writing
wk --dry-run apply
wk --dry-run sync
wk --dry-run mode docs/local sync

# Inspect scriptable state
wk status --json
```

New Git worktrees are explicit: after `git worktree add ...`, run `wk apply` to
materialize the managed files in that worktree.

## Modes

- `ignore`: keep the path known to `wk`, but do not manage it.
- `link`: create a symlink in each linked worktree pointing to the source copy in
  the main worktree.
- `copy`: copy from the main worktree when the destination is missing; later
  worktree edits are local.
- `sync`: copy in both directions through the main worktree as the hub.

Examples:

```bash
wk mode .claude link
wk mode AGENTS.local.md copy
wk mode docs/local sync --sync-policy manual --conflict-policy ask
wk mode docs/local sync --choice source
wk sync
```

`sync` is per entry inside directories. It copies source-only additions to the
worktree, worktree-only additions back to source, refreshes identical convergent
changes, and leaves conflicting entries untouched unless a conflict policy says
otherwise. Directory sync never replaces a whole directory to resolve one file.

## Status

`wk status --json` is intended for scripts:

- exit `0`: clean
- exit `1`: drift exists but no conflicts
- exit `2`: at least one conflict exists
- exit `3+`: command or repository error

Pretty status is available with `wk status`.

`wk mode` uses conservative defaults. Use `--choice source` or
`--choice worktree` when a transition needs an explicit canonical side.

## Safety Model

- `.wk/` is appended to `.gitignore` by `wk init`.
- Config paths are concrete repository-relative paths; globs are discovery input
  only and are expanded before persistence.
- Non-ignored paths are rejected unless explicitly forced through add/discovery.
- The main worktree is the source copy and is never an apply destination.
- Directory copy is overlay-only: destination-only files are not deleted.
- Symlinks inside copied directories are preserved as symlinks.
- Foreign symlinks are not repaired automatically.
- `newer` conflict resolution uses mtimes and prints an unsafe warning.
- `gc` previews old backups by default; deletion requires `--force`.

V1 does not install hooks by default, does not run a daemon, and does not support
bare repositories.

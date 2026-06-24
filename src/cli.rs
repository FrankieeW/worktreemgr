use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "wk",
    version,
    about = "Manage gitignored AI/local files across git worktrees",
    arg_required_else_help = true
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Avoid prompts and fail if a decision would require interaction"
    )]
    pub non_interactive: bool,

    #[arg(
        long,
        global = true,
        help = "Print planned operations without mutating files"
    )]
    pub dry_run: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Configure ignored local files for this repository")]
    Init,
    #[command(about = "Add a concrete path or discovery glob to the managed set")]
    Add {
        #[arg(help = "Concrete repository-relative path or discovery glob")]
        path: String,
        #[arg(long, help = "Allow managing a path that is not ignored by git")]
        force: bool,
    },
    #[command(about = "Materialize configured paths into one or all worktrees")]
    Apply {
        #[arg(help = "Optional worktree identity or path")]
        worktree: Option<String>,
    },
    #[command(about = "Show managed path drift and conflicts")]
    Status {
        #[arg(long, help = "Emit machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Synchronize sync-mode paths")]
    Sync {
        #[arg(help = "Optional managed path selector")]
        path: Option<String>,
        #[arg(help = "Optional worktree selector")]
        worktree: Option<String>,
    },
    #[command(about = "Change a managed path's mode")]
    Mode {
        #[arg(help = "Managed path to change")]
        path: String,
        #[arg(help = "Target mode")]
        mode: CliMode,
        #[arg(long, value_enum, help = "Sync policy when changing into sync mode")]
        sync_policy: Option<CliSyncPolicy>,
        #[arg(
            long,
            value_enum,
            help = "Conflict policy when changing into sync mode"
        )]
        conflict_policy: Option<CliConflictPolicy>,
        #[arg(
            long,
            value_enum,
            help = "Transition choice for mode changes that need reconciliation"
        )]
        choice: Option<CliTransitionChoice>,
    },
    #[command(about = "Remove state for deleted worktrees")]
    Prune,
    #[command(about = "Preview or remove old backups")]
    Gc {
        #[arg(long, help = "Remove backups after confirmation")]
        force: bool,
        #[arg(long, help = "Retention duration, such as 30d")]
        older_than: Option<String>,
        #[arg(
            long,
            help = "Keep the newest N backups even if older than the retention duration"
        )]
        keep: Option<usize>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CliMode {
    #[value(help = "Do not manage the path")]
    Ignore,
    #[value(help = "Symlink destinations to the source copy")]
    Link,
    #[value(help = "Copy once; allow worktree-local edits")]
    Copy,
    #[value(help = "Copy and allow later bidirectional sync")]
    Sync,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CliSyncPolicy {
    #[value(help = "Sync only when wk sync is run")]
    Manual,
    #[value(help = "Sync during apply and selected transitions")]
    Auto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CliConflictPolicy {
    #[value(help = "Ask before resolving")]
    Ask,
    #[value(help = "Source copy wins")]
    Source,
    #[value(help = "Worktree copy wins")]
    Worktree,
    #[value(help = "Newer mtime wins; unsafe and never a default")]
    Newer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CliTransitionChoice {
    #[value(help = "Use the conservative default for the transition")]
    Default,
    #[value(help = "Use the source copy as canonical")]
    Source,
    #[value(help = "Use the worktree copy as canonical")]
    Worktree,
    #[value(help = "Skip changes that would need reconciliation")]
    Skip,
}

use crate::{
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    error::WkError,
};

pub trait Prompter {
    fn select_mode(&self, path: &ManagedPath) -> Result<Mode, WkError>;
    fn select_sync_policy(&self, path: &ManagedPath) -> Result<SyncPolicy, WkError>;
    fn select_conflict_policy(&self, path: &ManagedPath) -> Result<ConflictPolicy, WkError>;
    fn confirm(&self, message: &str, default: bool) -> Result<bool, WkError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CliclackPrompter {
    non_interactive: bool,
}

impl CliclackPrompter {
    pub const fn new(non_interactive: bool) -> Self {
        Self { non_interactive }
    }

    fn require_interactive(self) -> Result<(), WkError> {
        if self.non_interactive {
            return Err(WkError::message(
                "interaction required but --non-interactive was supplied".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Prompter for CliclackPrompter {
    fn select_mode(&self, path: &ManagedPath) -> Result<Mode, WkError> {
        self.require_interactive()?;
        Ok(cliclack::select(format!("Mode for {path}"))
            .item(Mode::Ignore, "ignore", "track only; do not manage")
            .item(Mode::Link, "link", "symlink each worktree to source")
            .item(Mode::Copy, "copy", "copy once; worktree-local changes")
            .item(Mode::Sync, "sync", "copy and allow bidirectional sync")
            .initial_value(Mode::Ignore)
            .interact()?)
    }

    fn select_sync_policy(&self, path: &ManagedPath) -> Result<SyncPolicy, WkError> {
        self.require_interactive()?;
        Ok(cliclack::select(format!("Sync policy for {path}"))
            .item(
                SyncPolicy::Manual,
                "manual",
                "sync only when wk sync is run",
            )
            .item(SyncPolicy::Auto, "auto", "sync during apply")
            .initial_value(SyncPolicy::Manual)
            .interact()?)
    }

    fn select_conflict_policy(&self, path: &ManagedPath) -> Result<ConflictPolicy, WkError> {
        self.require_interactive()?;
        let policy = cliclack::select(format!("Conflict policy for {path}"))
            .item(ConflictPolicy::Ask, "ask", "prompt before resolving")
            .item(ConflictPolicy::Source, "source", "source copy wins")
            .item(ConflictPolicy::Worktree, "worktree", "worktree copy wins")
            .item(ConflictPolicy::Newer, "newer", "unsafe mtime comparison")
            .initial_value(ConflictPolicy::Ask)
            .interact()?;
        if policy.requires_warning() {
            cliclack::log::warning("newer uses mtimes and may pick the wrong side")?;
        }
        Ok(policy)
    }

    fn confirm(&self, message: &str, default: bool) -> Result<bool, WkError> {
        self.require_interactive()?;
        Ok(cliclack::confirm(message)
            .initial_value(default)
            .interact()?)
    }
}

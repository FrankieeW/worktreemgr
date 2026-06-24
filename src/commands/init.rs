use std::process::ExitCode;

use crate::{
    atomic::ensure_private_dir,
    commands::{load_config_or_default, prompt_path_config, save_config},
    discovery::{DiscoveryOptions, discover_candidates},
    error::WkError,
    git_repo::RepoContext,
    ui::Prompter,
};

pub fn run(ctx: &RepoContext, prompter: &dyn Prompter) -> Result<ExitCode, WkError> {
    ensure_private_dir(&ctx.control_dir)?;
    append_control_dir_ignore(ctx)?;
    let mut config = load_config_or_default(ctx)?;
    let candidates = discover_candidates(
        ctx,
        DiscoveryOptions {
            include_defaults: true,
            force: false,
        },
    )?;
    for candidate in candidates {
        if config
            .paths
            .iter()
            .any(|path_config| path_config.path == candidate.path)
        {
            continue;
        }
        config
            .paths
            .push(prompt_path_config(candidate.path, prompter)?);
    }
    save_config(ctx, &config)?;
    Ok(ExitCode::SUCCESS)
}

fn append_control_dir_ignore(ctx: &RepoContext) -> Result<(), WkError> {
    let gitignore = ctx.main_worktree.join(".gitignore");
    let existing = match std::fs::read_to_string(&gitignore) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    if existing.lines().any(|line| line.trim() == ".wk/") {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".wk/\n");
    std::fs::write(gitignore, next)?;
    Ok(())
}

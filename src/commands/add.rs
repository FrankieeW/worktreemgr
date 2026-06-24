use std::process::ExitCode;

use crate::{
    commands::{load_config_or_default, prompt_path_config, save_config},
    discovery::expand_explicit_or_glob,
    error::WkError,
    git_repo::RepoContext,
    ui::Prompter,
};

pub fn run(
    ctx: &RepoContext,
    prompter: &dyn Prompter,
    input: &str,
    force: bool,
) -> Result<ExitCode, WkError> {
    let mut config = load_config_or_default(ctx)?;
    for discovered in expand_explicit_or_glob(ctx, input, force)? {
        if config
            .paths
            .iter()
            .any(|path_config| path_config.path == discovered.path)
        {
            continue;
        }
        config
            .paths
            .push(prompt_path_config(discovered.path, prompter)?);
    }
    save_config(ctx, &config)?;
    Ok(ExitCode::SUCCESS)
}

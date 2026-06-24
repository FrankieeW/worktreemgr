fn main() -> color_eyre::Result<std::process::ExitCode> {
    color_eyre::install()?;
    Ok(wk::run()?)
}

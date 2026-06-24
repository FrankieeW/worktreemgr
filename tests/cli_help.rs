use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_all_v1_commands() -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::cargo_bin("wk")?;
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"))
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("apply"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("mode"))
        .stdout(predicate::str::contains("prune"))
        .stdout(predicate::str::contains("gc"));
    Ok(())
}

#[test]
fn rejects_unknown_subcommand() -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::cargo_bin("wk")?;
    command
        .arg("made-up-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
    Ok(())
}

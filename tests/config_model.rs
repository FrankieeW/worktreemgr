use camino::Utf8Path;
use tempfile::tempdir;
use wk::{
    config::{Config, PathConfig, load_config, save_config_atomic},
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
};

#[test]
fn toml_roundtrips_concrete_managed_paths() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let config_path = utf8_path(temp.path())?.join("config.toml");
    let config = Config {
        version: 1,
        default_sync_policy: SyncPolicy::Manual,
        default_conflict_policy: ConflictPolicy::Ask,
        paths: vec![
            PathConfig {
                path: ManagedPath::parse(".claude")?,
                mode: Mode::Link,
                sync_policy: None,
                conflict_policy: None,
            },
            PathConfig {
                path: ManagedPath::parse("docs/local")?,
                mode: Mode::Sync,
                sync_policy: Some(SyncPolicy::Manual),
                conflict_policy: Some(ConflictPolicy::Ask),
            },
            PathConfig {
                path: ManagedPath::parse("AGENTS.local.md")?,
                mode: Mode::Copy,
                sync_policy: None,
                conflict_policy: None,
            },
        ],
    };

    save_config_atomic(&config_path, &config)?;
    let loaded = load_config(&config_path)?;

    assert_eq!(loaded, config);
    Ok(())
}

#[test]
fn persisted_config_rejects_unsafe_or_pattern_paths() {
    for input in [
        "/abs/path",
        "../escape",
        "nested/../escape",
        ".git/config",
        ".wk/state",
        "*.local.*",
        "",
    ] {
        assert!(ManagedPath::parse(input).is_err(), "{input} should fail");
    }
}

#[test]
fn newer_policy_parses_but_requires_warning() -> Result<(), Box<dyn std::error::Error>> {
    let config_text = r#"
version = 1
default_sync_policy = "manual"
default_conflict_policy = "newer"

[[paths]]
path = ".claude"
mode = "sync"
sync_policy = "auto"
conflict_policy = "newer"
"#;
    let temp = tempdir()?;
    let config_path = utf8_path(temp.path())?.join("config.toml");
    std::fs::write(&config_path, config_text)?;

    let config = load_config(&config_path)?;

    assert!(config.default_conflict_policy.requires_warning());
    assert!(
        config.paths[0]
            .conflict_policy
            .unwrap_or(config.default_conflict_policy)
            .requires_warning()
    );
    assert!(!ConflictPolicy::Ask.requires_warning());
    Ok(())
}

fn utf8_path(path: &std::path::Path) -> Result<&Utf8Path, Box<dyn std::error::Error>> {
    Utf8Path::from_path(path).ok_or_else(|| "temporary path is not UTF-8".into())
}

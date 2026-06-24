use std::io::Write as _;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{ConflictPolicy, ManagedPath, Mode, SyncPolicy},
    error::WkError,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub default_sync_policy: SyncPolicy,
    pub default_conflict_policy: ConflictPolicy,
    #[serde(default)]
    pub paths: Vec<PathConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathConfig {
    pub path: ManagedPath,
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_policy: Option<SyncPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_policy: Option<ConflictPolicy>,
}

pub fn load_config(path: &Utf8Path) -> Result<Config, WkError> {
    let contents = std::fs::read_to_string(path)?;
    let config = toml::from_str::<Config>(&contents)?;
    config.validate()?;
    Ok(config)
}

pub fn save_config_atomic(path: &Utf8Path, config: &Config) -> Result<(), WkError> {
    config.validate()?;
    let parent = path
        .parent()
        .ok_or_else(|| WkError::message(format!("config path has no parent: {path}")))?;
    let contents = toml::to_string_pretty(config)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(contents.as_bytes())?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|error| WkError::Persist {
        path: path.to_path_buf(),
        source: error,
    })?;
    Ok(())
}

impl Config {
    pub fn validate(&self) -> Result<(), WkError> {
        if self.version != 1 {
            return Err(WkError::message(format!(
                "unsupported config version: {}",
                self.version
            )));
        }
        for path in &self.paths {
            ManagedPath::parse(path.path.as_str())?;
        }
        Ok(())
    }
}

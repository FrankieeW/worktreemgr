use std::fmt;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::error::WkError;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ignore,
    Link,
    Copy,
    Sync,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicy {
    Manual,
    Auto,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Ask,
    Source,
    Worktree,
    Newer,
}

impl ConflictPolicy {
    pub const fn requires_warning(self) -> bool {
        matches!(self, Self::Newer)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedPath(Utf8PathBuf);

impl ManagedPath {
    pub fn parse(input: &str) -> Result<Self, WkError> {
        let normalized = input.trim();
        validate_managed_path(normalized)?;
        Ok(Self(Utf8PathBuf::from(normalized)))
    }

    pub fn as_path(&self) -> &Utf8Path {
        self.0.as_path()
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn to_path_buf(&self) -> Utf8PathBuf {
        self.0.clone()
    }
}

impl fmt::Display for ManagedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ManagedPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ManagedPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(de::Error::custom)
    }
}

fn validate_managed_path(input: &str) -> Result<(), WkError> {
    if input.is_empty() {
        return Err(WkError::invalid_managed_path(input, "path cannot be empty"));
    }
    if has_glob_meta(input) {
        return Err(WkError::invalid_managed_path(
            input,
            "persisted paths must be concrete, not globs",
        ));
    }
    let path = Utf8Path::new(input);
    if path.is_absolute() {
        return Err(WkError::invalid_managed_path(
            input,
            "path must be repository-relative",
        ));
    }
    for segment in input.split('/') {
        if segment.is_empty() || segment == "." {
            return Err(WkError::invalid_managed_path(
                input,
                "path must be normalized",
            ));
        }
        if segment == ".." {
            return Err(WkError::invalid_managed_path(
                input,
                "path must not contain parent traversal",
            ));
        }
        if segment == ".git" || segment == ".wk" {
            return Err(WkError::invalid_managed_path(
                input,
                "nested .git and .wk paths are not managed",
            ));
        }
    }
    Ok(())
}

fn has_glob_meta(input: &str) -> bool {
    input
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
}

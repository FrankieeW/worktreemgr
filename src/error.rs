use camino::Utf8PathBuf;

pub type WkResult<T> = Result<T, WkError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WkError {
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(Utf8PathBuf),

    #[error("invalid managed path `{input}`: {reason}")]
    InvalidManagedPath { input: String, reason: &'static str },

    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("failed to parse TOML config")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("failed to serialize TOML config")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("failed to persist atomic file at {path}")]
    Persist {
        path: Utf8PathBuf,
        #[source]
        source: tempfile::PersistError,
    },

    #[error("{0}")]
    Message(String),
}

impl WkError {
    pub fn invalid_managed_path(input: &str, reason: &'static str) -> Self {
        Self::InvalidManagedPath {
            input: input.to_owned(),
            reason,
        }
    }

    pub const fn message(message: String) -> Self {
        Self::Message(message)
    }
}

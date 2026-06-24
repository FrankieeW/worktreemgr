use camino::{Utf8Path, Utf8PathBuf};

pub type WkResult<T> = Result<T, WkError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WkError {
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(String),

    #[error("invalid managed path `{input}`: {reason}")]
    InvalidManagedPath { input: String, reason: &'static str },

    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("directory walk error")]
    WalkDir(#[from] walkdir::Error),

    #[error("not a supported git repository: {reason}")]
    UnsupportedRepository { reason: String },

    #[error("git command failed in {cwd}: git {args}: {stderr}")]
    GitCommand {
        cwd: Utf8PathBuf,
        args: String,
        stderr: String,
    },

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
    pub const fn non_utf8_path(path: String) -> Self {
        Self::NonUtf8Path(path)
    }

    pub fn invalid_managed_path(input: &str, reason: &'static str) -> Self {
        Self::InvalidManagedPath {
            input: input.to_owned(),
            reason,
        }
    }

    pub const fn message(message: String) -> Self {
        Self::Message(message)
    }

    pub const fn unsupported_repository(reason: String) -> Self {
        Self::UnsupportedRepository { reason }
    }

    pub fn git_command(cwd: &Utf8Path, args: &[&str], stderr: String) -> Self {
        Self::GitCommand {
            cwd: cwd.to_path_buf(),
            args: args.join(" "),
            stderr,
        }
    }
}

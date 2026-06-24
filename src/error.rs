use camino::Utf8PathBuf;

pub type WkResult<T> = Result<T, WkError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WkError {
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(Utf8PathBuf),

    #[error("{0}")]
    Message(String),
}

//! The crate's boundary error (thiserror); `anyhow` is used inside.

/// Errors leaving `arch-facts`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The sqlite store failed.
    #[error("store: {0}")]
    Store(#[from] rusqlite::Error),
    /// A file under `.arch/` or the cache could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The path concerned.
        path: std::path::PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A committed `.arch/` file does not parse.
    #[error("{path}: {message}")]
    Format {
        /// The path concerned.
        path: std::path::PathBuf,
        /// What is wrong.
        message: String,
    },
    /// A fact stored as JSON does not decode.
    #[error("stored fact: {0}")]
    Json(#[from] serde_json::Error),
    /// A `git` invocation failed (the notes archive, ADR 0003).
    #[error("git {args}: {stderr}")]
    Git {
        /// The arguments passed to git.
        args: String,
        /// What git said.
        stderr: String,
    },
    /// Something else, with context.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result alias over [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn io(path: impl Into<std::path::PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
    pub(crate) fn format(path: impl Into<std::path::PathBuf>, message: impl ToString) -> Self {
        Error::Format {
            path: path.into(),
            message: message.to_string(),
        }
    }
}

use std::path::PathBuf;

use thiserror::Error;

/// Errors that abort registry discovery.
///
/// A single unreadable model file does not abort discovery: it becomes an
/// `unsupported` pack with an English `reason`. Only failures that make the
/// root itself unusable are returned here.
#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model root does not exist: {path}")]
    RootNotFound { path: PathBuf },

    #[error("model root is not a directory: {path}")]
    RootNotDirectory { path: PathBuf },

    #[error("failed to read directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to stat {path}: {source}")]
    Stat {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to serialize model registry: {0}")]
    Serialize(String),
}

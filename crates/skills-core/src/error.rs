use std::path::PathBuf;

/// Errors produced by `skills-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine the home directory")]
    NoHomeDir,

    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// The lockfile exists but cannot be used safely (merge conflict markers,
    /// unknown schema version, missing `skills` map). We refuse to overwrite
    /// it, because treating it as empty would silently drop every entry.
    #[error("lockfile {path} is not usable: {reason}")]
    InvalidLock { path: PathBuf, reason: String },

    #[error("invalid SKILL.md at {path}: {reason}")]
    InvalidSkill { path: PathBuf, reason: String },

    #[error("path {path} escapes its expected base directory {base}")]
    PathEscape { path: PathBuf, base: PathBuf },

    /// Data from skills.sh was not in the expected shape.
    #[error("unexpected data from skills.sh: {0}")]
    Parse(String),

    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

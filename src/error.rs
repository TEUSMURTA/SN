use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, SnError>;

#[derive(Debug, Error)]
pub enum SnError {
    #[error("I/O error while {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("unexpected end of input at byte {offset}: needed {requested} bytes")]
    UnexpectedEof { offset: u64, requested: usize },

    #[error("invalid {resource}: {reason}")]
    InvalidData {
        resource: &'static str,
        reason: String,
    },

    #[error("unsupported feature: {0}")]
    Unsupported(String),

    #[error("{resource} value {actual} exceeds limit {limit}")]
    LimitExceeded {
        resource: &'static str,
        actual: u64,
        limit: u64,
    },

    #[error("size overflow while calculating {resource}")]
    SizeOverflow { resource: &'static str },

    #[error("memory allocation failed for {resource}")]
    AllocationFailed { resource: &'static str },

    #[error("PNG encoding failed: {0}")]
    Png(#[from] png::EncodingError),

    #[error("glTF JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

impl SnError {
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }
}

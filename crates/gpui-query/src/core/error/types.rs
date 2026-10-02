use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::sanitize::sanitize_message;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryErrorKind {
    Cancelled,
    Response,
    Transport,
    Unknown,
}

impl std::fmt::Display for QueryErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::Response => write!(f, "response error"),
            Self::Transport => write!(f, "transport error"),
            Self::Unknown => write!(f, "unknown error"),
        }
    }
}

/// Implements [`std::error::Error`] for `?`/`anyhow` interop; the `Arc<str>`
/// message makes clones cheap in high-retry scenarios.
///
/// Messages are stored verbatim and may be serialized; use [`QueryError::sanitized`]
/// for server responses.
///
/// # Example
///
/// ```
/// use gpui_query::QueryError;
///
/// let err = QueryError::response("not found");
/// assert_eq!(err.to_string(), "response error: not found");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError {
    pub(super) kind: QueryErrorKind,
    pub(super) message: Arc<str>,
}

impl QueryError {
    pub fn new(kind: QueryErrorKind, message: impl Into<Arc<str>>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn cancelled(message: impl Into<Arc<str>>) -> Self {
        Self::new(QueryErrorKind::Cancelled, message)
    }

    pub fn response(message: impl Into<Arc<str>>) -> Self {
        Self::new(QueryErrorKind::Response, message)
    }

    pub fn transport(message: impl Into<Arc<str>>) -> Self {
        Self::new(QueryErrorKind::Transport, message)
    }

    pub fn unknown(message: impl Into<Arc<str>>) -> Self {
        Self::new(QueryErrorKind::Unknown, message)
    }

    pub fn kind(&self) -> QueryErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn message_arc(&self) -> &Arc<str> {
        &self.message
    }

    /// Redacts connection strings, bearer tokens, file paths, emails, and long
    /// hex runs; truncates to [`SANITIZE_MAX_LEN`](super::sanitize::SANITIZE_MAX_LEN) bytes.
    #[expect(
        rustdoc::private_intra_doc_links,
        reason = "the const stays internal; the link renders under --document-private-items"
    )]
    pub fn sanitized(&self) -> Self {
        let redacted = sanitize_message(&self.message);
        Self {
            kind: self.kind,
            message: redacted.into(),
        }
    }
}

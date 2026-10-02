//! Error types for query operations.
//! Messages are stored verbatim and may reach logs and serialized output;
//! use [`QueryError::sanitized`] on server responses.

mod convert;
pub(crate) mod sanitize;
mod serde;
mod types;

pub use types::{QueryError, QueryErrorKind};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::sanitize::SANITIZE_MAX_LEN;
    use super::types::{QueryError, QueryErrorKind};

    #[test]
    fn sanitized_redacts_connection_strings() {
        let err = QueryError::transport("connection to postgres://user:pass@host/db failed");
        let clean = err.sanitized();
        assert!(!clean.message().contains("user:pass@host"));
        assert!(clean.message().contains("[REDACTED_CONNECTION]"));
    }

    #[test]
    fn sanitized_redacts_bearer_tokens() {
        let err = QueryError::response("auth failed: bearer abc123token");
        let clean = err.sanitized();
        assert!(!clean.message().contains("abc123token"));
        assert!(clean.message().contains("[REDACTED_TOKEN]"));
    }

    #[test]
    fn sanitized_redacts_file_paths() {
        let err = QueryError::unknown("error reading /home/user/secret.key");
        let clean = err.sanitized();
        assert!(!clean.message().contains("/home/user/secret.key"));
        assert!(clean.message().contains("[REDACTED_PATH]"));
    }

    #[test]
    fn sanitized_redacts_emails() {
        let err = QueryError::response("user admin@example.com not found");
        let clean = err.sanitized();
        assert!(!clean.message().contains("admin@example.com"));
        assert!(clean.message().contains("[REDACTED_EMAIL]"));
    }

    #[test]
    fn sanitized_redacts_long_hex() {
        let err = QueryError::response("key a1b2c3d4e5f6a1b2c3d4e5f6a1b2 rejected");
        let clean = err.sanitized();
        assert!(!clean.message().contains("a1b2c3d4e5f6a1b2c3d4e5f6a1b2"));
        assert!(clean.message().contains("[REDACTED_HEX]"));
    }

    #[test]
    fn sanitized_truncates_long_messages() {
        let long_msg = "x".repeat(600);
        let err = QueryError::unknown(&*long_msg);
        let clean = err.sanitized();
        assert!(clean.message().len() <= SANITIZE_MAX_LEN + "...[truncated]".len());
        assert!(clean.message().ends_with("...[truncated]"));
    }

    #[test]
    fn sanitized_preserves_kind() {
        let err = QueryError::cancelled("aborted");
        assert_eq!(err.sanitized().kind(), QueryErrorKind::Cancelled);
    }

    #[test]
    fn clone_is_cheap() {
        let err = QueryError::response("test error");
        let err2 = err.clone();
        assert!(Arc::ptr_eq(&err.message, &err2.message));
    }
}

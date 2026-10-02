use super::types::QueryError;

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind(), self.message())
    }
}

impl std::error::Error for QueryError {}

impl AsRef<str> for QueryError {
    fn as_ref(&self) -> &str {
        self.message()
    }
}

impl AsRef<std::sync::Arc<str>> for QueryError {
    fn as_ref(&self) -> &std::sync::Arc<str> {
        &self.message
    }
}

impl From<String> for QueryError {
    /// Always maps to `Unknown` because the category cannot be recovered from
    /// a plain string; use `transport`/`response`/`cancelled` for typed errors.
    fn from(value: String) -> Self {
        Self::unknown(value)
    }
}

impl From<&str> for QueryError {
    fn from(value: &str) -> Self {
        Self::unknown(value)
    }
}

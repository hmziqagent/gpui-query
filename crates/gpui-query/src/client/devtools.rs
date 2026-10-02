#[cfg(feature = "persist")]
use std::any::TypeId;

use crate::core::{MutationStatus, QueryStatus};

#[derive(Clone, Debug)]
pub struct QueryDiagnostic {
    /// Key path, e.g. "users::42::posts".
    pub key: String,
    pub status: QueryStatus,
    pub cache_policy: String,
    pub cache_age_ms: Option<u64>,
    pub cache_hits: u64,
    pub retry_count: u32,
}

#[derive(Clone, Debug)]
pub struct MutationDiagnostic {
    pub key: Option<String>,
    pub status: MutationStatus,
    pub retry_count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ClientDiagnostic {
    pub query_count: usize,
    pub mutation_count: usize,
    pub queries: Vec<QueryDiagnostic>,
    pub mutations: Vec<MutationDiagnostic>,
}

#[cfg(feature = "persist")]
#[derive(Clone, Debug)]
pub struct DehydratedEntry {
    /// Key path, e.g. "users::42::posts".
    pub key: String,
    /// Matches entries to concrete types during hydration.
    pub type_id: TypeId,
    /// "query", "infinite", or "mutation".
    pub kind: &'static str,
}

/// Type-erased: callers that know the concrete types iterate `entries` and
/// call [`set_query_data`](super::QueryClient::set_query_data) per entry.
///
/// # Example
///
/// ```
/// use gpui_query::client::{DehydratedState, DehydratedEntry};
/// use std::any::TypeId;
///
/// // DehydratedState can be constructed directly
/// let state = DehydratedState::default();
/// assert!(state.entries.is_empty());
///
/// // Entries can be created and added
/// let entries = vec![DehydratedEntry {
///     key: "users".to_string(),
///     type_id: TypeId::of::<(String, String)>(),
///     kind: "query",
/// }];
/// let state = DehydratedState { entries };
/// assert_eq!(state.entries.len(), 1);
/// ```
#[cfg(feature = "persist")]
#[derive(Clone, Debug, Default)]
pub struct DehydratedState {
    pub entries: Vec<DehydratedEntry>,
}

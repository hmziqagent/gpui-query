//! Fetcher result wrapper for "server wins" caching: a `Some` policy
//! overrides the caller's per-query one. `meta` exists only under `persist`
//! so core stays serde_json-free.

use crate::core::policy::CachePolicy;
#[cfg(feature = "persist")]
use serde_json::Value as JsonValue;

/// `cache_policy: None` keeps the caller's per-query policy, matching a plain
/// `Result<T, E>` fetcher.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    pub data: T,
    /// Server-derived policy; `None` keeps the caller's.
    pub cache_policy: Option<CachePolicy>,
    /// Flows into `PersistedEntry::meta` for cold-start rehydration (e.g. HTTP `CacheMeta`).
    #[cfg(feature = "persist")]
    pub meta: Option<JsonValue>,
}

impl<T> Fetched<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            cache_policy: None,
            #[cfg(feature = "persist")]
            meta: None,
        }
    }

    pub fn with_policy(data: T, policy: CachePolicy) -> Self {
        Self {
            data,
            cache_policy: Some(policy),
            #[cfg(feature = "persist")]
            meta: None,
        }
    }

    #[cfg(feature = "persist")]
    pub fn with_meta(mut self, meta: JsonValue) -> Self {
        self.meta = Some(meta);
        self
    }
}

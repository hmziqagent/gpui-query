//! GPUI `QueryClient`: a [`Global`] registry managing type-partitioned
//! buckets for queries, mutations, and observers, with bulk operations
//! (invalidation, cancellation, GC) on top.

mod bucket;
mod devtools;
mod erased;
mod infinite_bucket;
mod infinite_mutation_ops;
mod lifecycle;
mod mutation_bucket;
#[cfg(feature = "persist")]
mod mutation_signal;
mod observer;
#[cfg(feature = "persist")]
mod persist;
mod prepared_fetch;
mod time;

pub use bucket::QueryBucket;
pub use devtools::{ClientDiagnostic, MutationDiagnostic, QueryDiagnostic};
#[cfg(feature = "persist")]
pub use devtools::{DehydratedEntry, DehydratedState};
#[cfg(feature = "persist")]
pub use erased::QueryPersister;
pub use infinite_bucket::InfiniteQueryBucket;
pub use mutation_bucket::MutationBucket;
#[cfg(feature = "persist")]
pub use mutation_signal::CacheMutation;
pub use observer::{
    InfiniteQueryObserver, MutationObserver, ObservableResource, Observer, ObserverConfig,
    QueryObserver,
};
#[cfg(feature = "persist")]
pub use persist::{
    NoopPersister, PERSIST_VERSION, PersistError, PersistFilter, PersistHandle, PersistOptions,
    PersistSnapshot, PersistedEntry, Persister, SerializerRegistry, hydrate,
};
pub use prepared_fetch::PreparedFetch;
pub use time::current_time_ms;

use std::any::TypeId;

use ahash::AHashMap;
use gpui::{App, Entity, Global};

use crate::client::bucket::shared::GC_INTERVAL;
use crate::client::bucket::types::MIN_GC_TIME_MS;
use crate::client::erased::{ErasedBucket, ErasedMutationBucket};
use crate::core::{CachePolicy, QueryKey, QueryResource, RequestPolicy};

/// Set once via `cx.set_global(QueryClient::default())`, read from any
/// component via `cx.global::<QueryClient>()`.
pub struct QueryClient {
    pub(crate) buckets: AHashMap<TypeId, Box<dyn ErasedBucket>>,
    pub(crate) infinite_buckets: AHashMap<TypeId, Box<dyn ErasedBucket>>,
    pub(crate) mutation_buckets: AHashMap<TypeId, Box<dyn ErasedMutationBucket>>,
    pub(crate) default_cache_policy: CachePolicy,
    pub(crate) default_request_policy: RequestPolicy,
    pub(crate) gc_time_ms: u64,
    /// Populated by `register_serializer::<T, E>` (value-carrying persistence path).
    #[cfg(feature = "persist")]
    pub(crate) serializers: Option<crate::client::persist::SerializerRegistry>,
    /// Populated by `register_deserializer::<T, E>`, consumed by [`hydrate`].
    #[cfg(feature = "persist")]
    pub(crate) deserializers: Option<crate::client::persist::DeserializerRegistry>,
    /// Per-key metadata from `Fetched::meta`, surfaced into
    /// `PersistedEntry::meta`; pruned of evicted keys by GC.
    #[cfg(feature = "persist")]
    pub(crate) persisted_meta:
        Option<std::collections::HashMap<crate::core::QueryKey, serde_json::Value>>,
    op_count: u64,
    /// Wall-clock ms of the last sweep; `0` means "not yet seeded" (the first
    /// reach seeds it and skips that sweep).
    last_gc_ms: u64,
}

impl Global for QueryClient {}

impl Default for QueryClient {
    /// `gc_time_ms` defaults to 300_000 (5 minutes).
    fn default() -> Self {
        Self {
            buckets: AHashMap::new(),
            infinite_buckets: AHashMap::new(),
            mutation_buckets: AHashMap::new(),
            default_cache_policy: CachePolicy::default(),
            default_request_policy: RequestPolicy::default(),
            gc_time_ms: 300_000,
            #[cfg(feature = "persist")]
            serializers: None,
            #[cfg(feature = "persist")]
            deserializers: None,
            #[cfg(feature = "persist")]
            persisted_meta: None,
            op_count: 0,
            last_gc_ms: 0,
        }
    }
}

impl QueryClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_policies(
        default_cache_policy: CachePolicy,
        default_request_policy: RequestPolicy,
    ) -> Self {
        Self {
            default_cache_policy,
            default_request_policy,
            ..Default::default()
        }
    }

    /// Values below 1000ms are clamped to 1000ms during GC to prevent
    /// aggressive eviction of all Idle/Failure resources on every GC pass.
    /// A value of 0 disables the opportunistic sweep only; an explicit
    /// [`gc`](Self::gc) still sweeps with the clamped floor.
    pub fn with_gc_time(mut self, gc_time_ms: u64) -> Self {
        self.gc_time_ms = gc_time_ms;
        self
    }

    /// Captured from a fetcher's `Fetched::meta` at completion, surfaced into
    /// `PersistedEntry::meta`. `persist` feature only.
    #[cfg(feature = "persist")]
    pub(crate) fn record_meta(&mut self, key: crate::core::QueryKey, meta: serde_json::Value) {
        self.persisted_meta
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(key, meta);
    }

    /// Runs GC every `GC_INTERVAL` operations, at most once per
    /// `MIN_GC_TIME_MS`; `gc_time_ms` of 0 skips the opportunistic sweep.
    fn maybe_opportunistic_gc(&mut self, cx: &App) {
        if self.gc_time_ms == 0 {
            return;
        }
        self.op_count = self.op_count.wrapping_add(1);
        if !self.op_count.is_multiple_of(GC_INTERVAL as u64) {
            return;
        }
        let now_ms = current_time_ms();
        if self.last_gc_ms == 0 {
            self.last_gc_ms = now_ms;
            return;
        }
        if now_ms.saturating_sub(self.last_gc_ms) < MIN_GC_TIME_MS {
            return;
        }
        self.last_gc_ms = now_ms;
        self.gc_with_time(now_ms, cx);
    }

    pub fn resource<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl Into<QueryKey>,
        cx: &mut App,
    ) -> Entity<QueryResource<T, E>> {
        self.resource_with_policies::<T, E>(
            key,
            self.default_cache_policy,
            self.default_request_policy,
            cx,
        )
    }

    pub fn resource_with_policies<
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    >(
        &mut self,
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> Entity<QueryResource<T, E>> {
        let type_id = TypeId::of::<(T, E)>();
        let bucket = self
            .buckets
            .entry(type_id)
            .or_insert_with(|| Box::new(QueryBucket::<T, E>::new()));

        let typed = Self::bucket_or_recreate(bucket, QueryBucket::<T, E>::new);
        let entity = typed.get_or_create(key.into(), cache_policy, request_policy, cx);
        self.maybe_opportunistic_gc(cx);
        entity
    }

    /// Mints the request id in the same bucket lookup, skipping a second
    /// TypeId+key hash of a follow-up `next_request_id_for_key`.
    fn resource_with_request_id<
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    >(
        &mut self,
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> (Entity<QueryResource<T, E>>, crate::core::RequestId) {
        let type_id = TypeId::of::<(T, E)>();
        let bucket = self
            .buckets
            .entry(type_id)
            .or_insert_with(|| Box::new(QueryBucket::<T, E>::new()));
        let typed = Self::bucket_or_recreate(bucket, QueryBucket::<T, E>::new);
        let (entity, request_id) =
            typed.get_or_create_with_request_id(key.into(), cache_policy, request_policy, cx);
        self.maybe_opportunistic_gc(cx);
        (entity, request_id)
    }

    pub fn all_queries<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(
        &self,
    ) -> Vec<Entity<QueryResource<T, E>>> {
        let type_id = TypeId::of::<(T, E)>();
        self.buckets
            .get(&type_id)
            .and_then(|b| b.as_any().downcast_ref::<QueryBucket<T, E>>())
            .map(|b| b.all_entities())
            .unwrap_or_default()
    }

    pub fn query<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(
        &self,
        key: &QueryKey,
    ) -> Option<Entity<QueryResource<T, E>>> {
        let type_id = TypeId::of::<(T, E)>();
        self.buckets
            .get(&type_id)
            .and_then(|b| b.as_any().downcast_ref::<QueryBucket<T, E>>())
            .and_then(|b| b.get(key))
    }

    /// Returns `None` if no bucket entry exists for the key; the sequencer is
    /// persistent, so IDs stay monotonic for the entry's lifetime.
    pub fn next_request_id_for_key<
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    >(
        &mut self,
        key: &QueryKey,
    ) -> Option<crate::core::RequestId> {
        let type_id = TypeId::of::<(T, E)>();
        let bucket = self.buckets.get_mut(&type_id)?;
        let typed = Self::bucket_or_recreate(bucket, QueryBucket::<T, E>::new);
        typed.sequencer_mut(key).map(|seq| seq.next_request())
    }

    /// Recreates the bucket in place on a downcast mismatch (unreachable
    /// while `TypeId` keys are sound) instead of panicking.
    fn bucket_or_recreate<B>(
        bucket: &mut Box<dyn ErasedBucket>,
        fresh: impl FnOnce() -> B,
    ) -> &mut B
    where
        B: ErasedBucket + 'static,
    {
        if bucket.as_any_mut().downcast_mut::<B>().is_none() {
            eprintln!(
                "QueryClient: type mismatch in bucket downcast for {}. \
                 Replacing with a fresh bucket.",
                std::any::type_name::<B>()
            );
            *bucket = Box::new(fresh());
        }
        bucket
            .as_any_mut()
            .downcast_mut::<B>()
            .expect("downcast succeeds after bucket_or_recreate replaced the box")
    }

    /// Returns `None` if no resource exists for the key, the entity was
    /// collected, or the resource has not completed a fetch.
    pub fn get_query_data<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(
        &self,
        key: &QueryKey,
        cx: &App,
    ) -> Option<T> {
        let entity = self.query::<T, E>(key)?;
        entity.read_with(cx, |resource, _| resource.data().cloned())
    }

    /// Zero-clone counterpart to [`get_query_data`](Self::get_query_data):
    /// `f` receives `&T` for the duration of the call. Same `None`
    /// conditions.
    pub fn with_query_data<
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
        R,
    >(
        &self,
        key: &QueryKey,
        cx: &App,
        f: impl FnOnce(&T) -> R,
    ) -> Option<R> {
        let entity = self.query::<T, E>(key)?;
        entity.read_with(cx, |resource, _| resource.data().map(f))
    }

    /// Creates the resource if absent; the previous data is kept for
    /// `rollback_to_previous()` and the resource's status and timestamp are
    /// unchanged.
    pub fn set_query_data<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static>(
        &mut self,
        key: impl Into<QueryKey>,
        data: T,
        cx: &mut App,
    ) {
        let key = key.into();
        let entity = self.resource::<T, E>(key, cx);
        entity.update(cx, |resource, cx| {
            resource.set_data(data);
            cx.notify();
            #[cfg(feature = "persist")]
            cx.default_global::<crate::client::CacheMutation>();
        });
    }
}

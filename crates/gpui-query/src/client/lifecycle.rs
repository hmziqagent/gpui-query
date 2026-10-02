//! Lifecycle operations on `QueryClient`: GC, diagnostics, serialization,
//! legacy persistence, and imperative fetch/prefetch.

use gpui::App;

use crate::client::devtools::ClientDiagnostic;
#[cfg(feature = "persist")]
use crate::client::devtools::{DehydratedEntry, DehydratedState};
use crate::client::prepared_fetch::PreparedFetch;
use crate::client::time::current_time_ms;
use crate::core::{CachePolicy, QueryKey, RequestPolicy};
#[cfg(feature = "persist")]
use crate::core::{MutationStatus, QueryStatus};

use super::QueryClient;
#[cfg(feature = "persist")]
use crate::client::erased::QueryPersister;

impl QueryClient {
    /// Calls `current_time_ms()` internally; use
    /// [`gc_with_time`](Self::gc_with_time) if you already hold a time value.
    pub fn gc(&mut self, cx: &App) {
        let now_ms = current_time_ms();
        self.gc_with_time(now_ms, cx);
    }

    /// Stamps `last_gc_ms`, so a manual GC debounces the next opportunistic
    /// sweep.
    pub fn gc_with_time(&mut self, now_ms: u64, cx: &App) {
        self.last_gc_ms = now_ms;
        for bucket in self.buckets.values_mut() {
            bucket.gc(now_ms, self.gc_time_ms, cx);
        }
        for bucket in self.infinite_buckets.values_mut() {
            bucket.gc(now_ms, self.gc_time_ms, cx);
        }
        for bucket in self.mutation_buckets.values_mut() {
            bucket.gc(now_ms, self.gc_time_ms, cx);
        }
        // Evicted keys' meta can never be collected again; drop it so churning keys can't grow the map.
        #[cfg(feature = "persist")]
        if let Some(meta) = self.persisted_meta.as_mut() {
            meta.retain(|key, _| {
                self.buckets.values().any(|b| b.contains_key(key))
                    || self.infinite_buckets.values().any(|b| b.contains_key(key))
            });
        }
    }

    /// Collected entities are skipped, so the aggregate counts are an upper
    /// bound on the returned vectors.
    pub fn diagnostics(&self, cx: &App) -> ClientDiagnostic {
        let now_ms = current_time_ms();
        let mut query_count = 0;
        let mut mutation_count = 0;
        for bucket in self.buckets.values() {
            query_count += bucket.count();
        }
        for bucket in self.infinite_buckets.values() {
            query_count += bucket.count();
        }
        for bucket in self.mutation_buckets.values() {
            mutation_count += bucket.count();
        }
        let mut queries = Vec::with_capacity(query_count);
        let mut mutations = Vec::with_capacity(mutation_count);

        for bucket in self.buckets.values() {
            bucket.collect_diagnostics_into(now_ms, cx, &mut queries);
        }
        for bucket in self.infinite_buckets.values() {
            bucket.collect_diagnostics_into(now_ms, cx, &mut queries);
        }
        for bucket in self.mutation_buckets.values() {
            bucket.collect_diagnostics_into(cx, &mut mutations);
        }

        ClientDiagnostic {
            query_count,
            mutation_count,
            queries,
            mutations,
        }
    }

    /// Only live `Success` resources are included. Full data serialization
    /// is type-specific: extract via
    /// [`get_query_data`](Self::get_query_data) and serialize externally.
    #[cfg(feature = "persist")]
    pub fn dehydrate(&self, cx: &App) -> DehydratedState {
        let cap = self.buckets.values().map(|b| b.count()).sum::<usize>()
            + self
                .infinite_buckets
                .values()
                .map(|b| b.count())
                .sum::<usize>()
            + self
                .mutation_buckets
                .values()
                .map(|b| b.count())
                .sum::<usize>();
        let mut entries = Vec::with_capacity(cap);

        fn push_status<S>(
            entries: &mut Vec<DehydratedEntry>,
            type_id: std::any::TypeId,
            pairs: impl IntoIterator<Item = (Option<String>, S)>,
            success: S,
            kind: &'static str,
        ) where
            S: PartialEq,
        {
            for (key, status) in pairs {
                if status == success
                    && let Some(key) = key
                {
                    entries.push(DehydratedEntry { key, type_id, kind });
                }
            }
        }

        // Scratch buffers drained per bucket: keys move into `entries` without cloning.
        let mut q_pairs: Vec<(String, QueryStatus)> = Vec::new();
        let mut m_pairs: Vec<(Option<String>, MutationStatus)> = Vec::new();

        for (type_id, bucket) in &self.buckets {
            bucket.collect_key_status_into(cx, &mut q_pairs);
            push_status(
                &mut entries,
                *type_id,
                q_pairs.drain(..).map(|(k, s)| (Some(k), s)),
                QueryStatus::Success,
                "query",
            );
        }
        for (type_id, bucket) in &self.infinite_buckets {
            bucket.collect_key_status_into(cx, &mut q_pairs);
            push_status(
                &mut entries,
                *type_id,
                q_pairs.drain(..).map(|(k, s)| (Some(k), s)),
                QueryStatus::Success,
                "infinite",
            );
        }
        for (type_id, bucket) in &self.mutation_buckets {
            bucket.collect_key_status_into(cx, &mut m_pairs);
            push_status(
                &mut entries,
                *type_id,
                m_pairs.drain(..),
                MutationStatus::Success,
                "mutation",
            );
        }

        DehydratedState { entries }
    }

    /// Hook point only: `DehydratedState` stores erased `type_id`s, so
    /// callers restore typed data themselves via
    /// `set_query_data::<T, E>()` for entries whose types they know.
    #[cfg(feature = "persist")]
    pub fn hydrate(&mut self, _state: DehydratedState, _cx: &mut App) {}

    #[cfg(feature = "persist")]
    pub fn persist(&self, persister: &dyn QueryPersister, cx: &App) {
        let state = self.dehydrate(cx);
        persister.save(state.entries);
    }

    /// Types are erased in the persister, so callers restore via
    /// `set_query_data` themselves. Associated fn: reads no client state.
    #[cfg(feature = "persist")]
    pub fn restore(persister: &dyn QueryPersister) -> Vec<DehydratedEntry> {
        persister.load()
    }

    /// Imperative fetch (TanStack `fetchQuery`): no observer is attached;
    /// the caller runs the fetcher and completes the request via
    /// `complete_success` / `complete_failure`. Returns `None` when the
    /// request policy ignored the start, leaving the in-flight fetcher in place.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use gpui_query::client::QueryClient;
    /// use gpui_query::core::QueryKey;
    /// # #[derive(Clone)]
    /// # struct UserData;
    /// # #[derive(Clone, Debug)]
    /// # struct QueryError;
    /// # fn _doc(client: &mut QueryClient, cx: &mut gpui::App) {
    ///
    /// if let Some(prepared) = client.prepare_fetch_query::<UserData, QueryError>(
    ///     QueryKey::from("user/42"),
    ///     cx,
    /// ) {
    ///     // prepared.entity, prepared.signal, and prepared.request_id are now available.
    ///     // Use cx.spawn() to run your async fetcher, then call
    ///     // prepared.complete_success(data, cx) or prepared.complete_failure(e, cx).
    /// }
    /// # }
    /// ```
    pub fn prepare_fetch_query<
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    >(
        &mut self,
        key: impl Into<QueryKey>,
        cx: &mut App,
    ) -> Option<PreparedFetch<T, E>> {
        let now_ms = current_time_ms();
        let (entity, request_id) = self.resource_with_request_id::<T, E>(
            key,
            self.default_cache_policy,
            self.default_request_policy,
            cx,
        );

        let (request_id, signal) = entity.update(cx, |resource, _| {
            match resource.begin_request_with_id(
                Some(request_id),
                now_ms,
                crate::core::QueryFetchMode::Force,
            ) {
                // Force still defers to IgnoreWhileLoading: the still-active id
                // belongs to a foreign fetcher, so binding would duplicate its completion.
                crate::core::QueryBeginResult::Started { .. } => {
                    let rid = resource.active_request_id()?;
                    let signal = resource.signal().cloned()?;
                    Some((rid, signal))
                }
                _ => None,
            }
        })?;

        Some(PreparedFetch {
            entity,
            request_id,
            signal,
            now_ms,
        })
    }

    /// Returns `None` on a fresh cache hit (read it via
    /// [`get_query_data`](Self::get_query_data)) or when the request policy
    /// ignored the start. No observer is attached; a later `use_query` with
    /// the same key finds the prefetched data.
    pub fn prepare_prefetch_query<
        T: Clone + Send + Sync + 'static,
        E: Clone + Send + Sync + 'static,
    >(
        &mut self,
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> Option<PreparedFetch<T, E>> {
        let now_ms = current_time_ms();
        let (entity, request_id) =
            self.resource_with_request_id::<T, E>(key, cache_policy, request_policy, cx);

        // Only Started and StaleCacheHit mean a fetch is actually wanted.
        let (request_id, signal) = entity.update(cx, |resource, _| {
            let active_before = resource.active_request_id();
            match resource.begin_request_with_id(
                Some(request_id),
                now_ms,
                crate::core::QueryFetchMode::Normal,
            ) {
                // IgnoreWhileLoading handing back the still-active id means a
                // revalidate is already running; prefetch must not duplicate it.
                crate::core::QueryBeginResult::StaleCacheHit {
                    request_id,
                    replaced_request_id: None,
                    ..
                } if Some(request_id) == active_before => None,
                crate::core::QueryBeginResult::Started { .. }
                | crate::core::QueryBeginResult::StaleCacheHit { .. } => {
                    let rid = resource.active_request_id()?;
                    let signal = resource.signal().cloned()?;
                    Some((rid, signal))
                }
                _ => None,
            }
        })?;

        Some(PreparedFetch {
            entity,
            request_id,
            signal,
            now_ms,
        })
    }
}

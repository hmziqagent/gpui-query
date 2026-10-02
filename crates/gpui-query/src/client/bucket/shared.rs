//! `ResourceBucket<R>` holds everything `QueryBucket` and
//! `InfiniteQueryBucket` do identically (get-or-create, eviction, GC, bulk
//! matching, diagnostics, persistence collection).

use ahash::AHashMap;
use gpui::{App, AppContext as _, Entity};

use crate::client::devtools::QueryDiagnostic;
#[cfg(feature = "persist")]
use crate::client::persist::{PersistFilter, PersistedEntry, SerializerRegistry};
use crate::core::{
    CachePolicy, InfiniteQueryResource, QueryKey, QueryKeyFilter, QueryResource, QueryStatus,
    RequestId, RequestPolicy,
};

use super::types::{BucketEntry, DEFAULT_MAX_ENTRIES, MIN_GC_TIME_MS, SUCCESS_GC_MULTIPLIER};
use crate::client::time::current_time_ms;

/// Runs GC every this many resource operations, so it fires in production
/// without anyone calling `gc()` by hand.
pub(crate) const GC_INTERVAL: usize = 64;

/// The resource surface `ResourceBucket` needs for both query kinds;
/// prefixed names keep the delegating impls unambiguous.
pub(crate) trait BucketResource {
    fn new_resource(
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
    ) -> Self;
    fn resource_status(&self) -> QueryStatus;
    fn resource_is_loading(&self) -> bool;
    fn resource_last_updated(&self) -> Option<u64>;
    fn resource_cache_policy(&self) -> CachePolicy;
    fn resource_request_policy(&self) -> RequestPolicy;
    fn set_resource_cache_policy(&mut self, policy: CachePolicy);
    fn set_resource_request_policy(&mut self, policy: RequestPolicy);
    fn resource_cache_age_ms(&self, now_ms: u64) -> Option<u64>;
    fn resource_cache_hits(&self) -> u64;
    fn resource_retry_count(&self) -> u32;
    /// Counts data writes; persistence collection skips entries whose epoch
    /// is unchanged since the last flush.
    #[cfg(feature = "persist")]
    fn resource_data_epoch(&self) -> u64;
    fn resource_invalidate(&mut self);
    fn resource_reset(&mut self);
    fn resource_cancel_inflight(&mut self);
}

impl<T: 'static, E: 'static> BucketResource for QueryResource<T, E> {
    fn new_resource(
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
    ) -> Self {
        Self::new(key, cache_policy, request_policy)
    }
    fn resource_status(&self) -> QueryStatus {
        self.status()
    }
    fn resource_is_loading(&self) -> bool {
        self.is_loading()
    }
    fn resource_last_updated(&self) -> Option<u64> {
        self.last_updated_at_ms()
    }
    fn resource_cache_policy(&self) -> CachePolicy {
        self.cache_policy()
    }
    fn resource_request_policy(&self) -> RequestPolicy {
        self.request_policy()
    }
    fn set_resource_cache_policy(&mut self, policy: CachePolicy) {
        self.set_cache_policy(policy);
    }
    fn set_resource_request_policy(&mut self, policy: RequestPolicy) {
        self.set_request_policy(policy);
    }
    fn resource_cache_age_ms(&self, now_ms: u64) -> Option<u64> {
        self.cache_age_ms(now_ms)
    }
    fn resource_cache_hits(&self) -> u64 {
        self.cache_hits()
    }
    fn resource_retry_count(&self) -> u32 {
        self.retry_count()
    }
    #[cfg(feature = "persist")]
    fn resource_data_epoch(&self) -> u64 {
        self.data_epoch()
    }
    fn resource_invalidate(&mut self) {
        self.invalidate();
    }
    fn resource_reset(&mut self) {
        self.reset();
    }
    fn resource_cancel_inflight(&mut self) {
        if let Some(signal) = self.signal() {
            signal.cancel();
        }
        self.mark_ignored_result();
    }
}

impl<T: 'static, E: 'static> BucketResource for InfiniteQueryResource<T, E> {
    fn new_resource(
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
    ) -> Self {
        Self::new(key, cache_policy, request_policy)
    }
    fn resource_status(&self) -> QueryStatus {
        self.status()
    }
    fn resource_is_loading(&self) -> bool {
        self.is_loading()
    }
    fn resource_last_updated(&self) -> Option<u64> {
        self.last_updated_at_ms()
    }
    fn resource_cache_policy(&self) -> CachePolicy {
        self.cache_policy()
    }
    fn resource_request_policy(&self) -> RequestPolicy {
        self.request_policy()
    }
    fn set_resource_cache_policy(&mut self, policy: CachePolicy) {
        self.set_cache_policy(policy);
    }
    fn set_resource_request_policy(&mut self, policy: RequestPolicy) {
        self.set_request_policy(policy);
    }
    fn resource_cache_age_ms(&self, now_ms: u64) -> Option<u64> {
        self.cache_age_ms(now_ms)
    }
    fn resource_cache_hits(&self) -> u64 {
        self.cache_hits()
    }
    fn resource_retry_count(&self) -> u32 {
        self.retry_count()
    }
    #[cfg(feature = "persist")]
    fn resource_data_epoch(&self) -> u64 {
        self.data_epoch()
    }
    fn resource_invalidate(&mut self) {
        self.invalidate();
    }
    fn resource_reset(&mut self) {
        self.reset();
    }
    fn resource_cancel_inflight(&mut self) {
        if let Some(signal) = self.signal() {
            signal.cancel();
        }
        self.mark_ignored_result();
    }
}

pub(crate) struct ResourceBucket<R> {
    pub(crate) entries: AHashMap<QueryKey, BucketEntry<R>>,
    pub(crate) max_entries: usize,
}

impl<R: BucketResource + 'static> ResourceBucket<R> {
    pub(crate) fn new() -> Self {
        Self {
            entries: AHashMap::new(),
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    pub(crate) fn get_or_create(
        &mut self,
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> Entity<R> {
        self.get_or_create_impl(key, cache_policy, request_policy, cx, false)
            .0
    }

    /// Mints the next `RequestId` from the entry's sequencer in the same
    /// lookup.
    pub(crate) fn get_or_create_with_request_id(
        &mut self,
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> (Entity<R>, RequestId) {
        let (entity, request_id) =
            self.get_or_create_impl(key, cache_policy, request_policy, cx, true);
        (
            entity,
            request_id.expect("impl inserts the entry before returning"),
        )
    }

    /// Live entries refresh differing policies in place; a dead weak
    /// reference is overwritten in place (length unchanged, no eviction),
    /// while a vacant insert at capacity evicts the oldest entry first.
    fn get_or_create_impl(
        &mut self,
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
        mint_request_id: bool,
    ) -> (Entity<R>, Option<RequestId>) {
        if let Some(entry) = self.entries.get_mut(&key) {
            if let Some(entity) = entry.entity.upgrade() {
                let (needs_update, last_updated, loading) = entity.read_with(cx, |resource, _| {
                    let needs_update = resource.resource_cache_policy() != cache_policy
                        || resource.resource_request_policy() != request_policy;
                    (
                        needs_update,
                        resource.resource_last_updated(),
                        resource.resource_is_loading(),
                    )
                });
                entry.last_updated_ms = last_updated;
                entry.loading = loading;
                entry.updated_at = current_time_ms();
                if needs_update {
                    entity.update(cx, |resource, _| {
                        resource.set_resource_cache_policy(cache_policy);
                        resource.set_resource_request_policy(request_policy);
                    });
                }
                let request_id = mint_request_id.then(|| entry.sequencer.next_request());
                return (entity, request_id);
            }
        } else if self.entries.len() >= self.max_entries {
            self.evict_oldest(cx);
        }

        let mut sequencer = crate::core::RequestSequencer::new();
        let request_id = mint_request_id.then(|| sequencer.next_request());
        let entity = cx.new(|_| R::new_resource(key.clone(), cache_policy, request_policy));
        self.entries.insert(
            key,
            BucketEntry {
                entity: entity.downgrade(),
                sequencer,
                updated_at: current_time_ms(),
                last_updated_ms: None,
                loading: false,
            },
        );
        (entity, request_id)
    }

    /// Dead entries are age-zero candidates; the mirror can be stale, so the
    /// winner is confirmed with one entity read.
    pub(crate) fn evict_oldest(&mut self, cx: &App) {
        loop {
            let target = self
                .entries
                .iter()
                .filter_map(|(key, entry)| {
                    if !entry.entity.is_upgradable() {
                        return Some((key, 0));
                    }
                    if entry.loading {
                        return None;
                    }
                    Some((key, entry.last_updated_ms.unwrap_or(entry.updated_at)))
                })
                .min_by_key(|&(_, age)| age);

            let Some((key, _)) = target else {
                return; // only reachable when every entry is loading
            };

            let key = key.clone();
            let still_loading = self
                .entries
                .get(&key)
                .and_then(|e| e.entity.upgrade())
                .map(|entity| entity.read(cx).resource_is_loading());

            match still_loading {
                Some(true) => {
                    if let Some(entry) = self.entries.get_mut(&key) {
                        entry.loading = true;
                    }
                }
                _ => {
                    self.entries.remove(&key);
                    return;
                }
            }
        }
    }

    pub(crate) fn get(&self, key: &QueryKey) -> Option<Entity<R>> {
        self.entries.get(key).and_then(|e| e.entity.upgrade())
    }

    pub(crate) fn sequencer_mut(
        &mut self,
        key: &QueryKey,
    ) -> Option<&mut crate::core::RequestSequencer> {
        self.entries.get_mut(key).map(|e| &mut e.sequencer)
    }

    pub(crate) fn all_entities(&self) -> Vec<Entity<R>> {
        self.entries
            .values()
            .filter_map(|e| e.entity.upgrade())
            .collect()
    }

    /// Collects matching entities up front, then runs `action` on each
    /// outside the map borrow. GPUI defers observer effects to the
    /// outermost update, so no action can re-enter this bucket mid-loop.
    pub(crate) fn for_each_matching_entry(
        &mut self,
        filter: &QueryKeyFilter,
        cx: &mut App,
        mut action: impl FnMut(&Entity<R>, &mut App),
    ) {
        let entities: Vec<Entity<R>> = self
            .entries
            .iter()
            .filter(|(key, _)| filter.matches(key))
            .filter_map(|(_, entry)| entry.entity.upgrade())
            .collect();

        for entity in &entities {
            action(entity, cx);
        }
    }

    /// Observers fire only from `cx.notify()` inside the update closure, so
    /// mutating bulk ops notify and the observer dedup suppresses no-change
    /// wakes.
    pub(crate) fn invalidate_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.for_each_matching_entry(filter, cx, |entity, cx| {
            // invalidate() only clears last_updated_at; skip the no-op update.
            let needs_invalidate = entity.read_with(cx, |r, _| r.resource_last_updated().is_some());
            if needs_invalidate {
                entity.update(cx, |resource, _| resource.resource_invalidate());
            }
        });
        self.touch_matching(filter);
    }

    pub(crate) fn reset_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.for_each_matching_entry(filter, cx, |entity, cx| {
            entity.update(cx, |resource, cx| {
                resource.resource_reset();
                cx.notify();
            });
        });
        self.touch_matching(filter);
    }

    /// Invalidated/reset resources lose their own `last_updated_at`, so the
    /// user action must restart the GC age baseline or live entries are evicted.
    fn touch_matching(&mut self, filter: &QueryKeyFilter) {
        let now_ms = current_time_ms();
        for (key, entry) in self.entries.iter_mut() {
            if filter.matches(key) {
                entry.updated_at = now_ms;
            }
        }
    }

    /// Gates on the authoritative `is_loading()` read: the entry mirror
    /// could be stale and skip an in-flight cancel.
    pub(crate) fn cancel_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.for_each_matching_entry(filter, cx, |entity, cx| {
            if entity.read_with(cx, |r, _| r.resource_is_loading()) {
                entity.update(cx, |resource, _| resource.resource_cancel_inflight());
            }
        });
    }

    pub(crate) fn remove_matching(&mut self, filter: &QueryKeyFilter) {
        self.entries.retain(|k, _| !filter.matches(k));
    }

    /// Loading always survives; `Success` survives while its cache policy
    /// can still serve it and until `SUCCESS_GC_MULTIPLIER * gc_time_ms`;
    /// `Idle`/`Failure`/`Cancelled` survive `gc_time_ms`. Entries without a
    /// completion timestamp age from the entry's `updated_at` baseline.
    pub(crate) fn gc(&mut self, now_ms: u64, gc_time_ms: u64, cx: &App) {
        let gc_threshold = gc_time_ms.max(MIN_GC_TIME_MS);
        let success_threshold = gc_threshold.saturating_mul(SUCCESS_GC_MULTIPLIER as u64);

        self.entries.retain(|_key, entry| {
            let Some(entity) = entry.entity.upgrade() else {
                return false;
            };
            let resource = entity.read(cx);

            let last_updated = resource.resource_last_updated();
            entry.last_updated_ms = last_updated;
            entry.loading = resource.resource_is_loading();

            if entry.loading {
                return true;
            }

            let status = resource.resource_status();
            let age_ms = last_updated
                .map(|updated| now_ms.saturating_sub(updated))
                .unwrap_or_else(|| now_ms.saturating_sub(entry.updated_at));

            if status == QueryStatus::Success {
                let cache_policy = resource.resource_cache_policy();
                if cache_policy.can_serve_stale() && !cache_policy.is_expired(age_ms) {
                    return true;
                }
                return age_ms < success_threshold;
            }

            if !matches!(
                status,
                QueryStatus::Idle | QueryStatus::Failure | QueryStatus::Cancelled
            ) {
                return true;
            }

            age_ms < gc_threshold
        });
    }

    pub(crate) fn collect_diagnostics_into(
        &self,
        now_ms: u64,
        cx: &App,
        out: &mut Vec<QueryDiagnostic>,
    ) {
        for (key, entry) in self.entries.iter() {
            let Some(entity) = entry.entity.upgrade() else {
                continue;
            };
            let resource = entity.read(cx);
            out.push(QueryDiagnostic {
                key: key.to_path(),
                status: resource.resource_status(),
                cache_policy: resource.resource_cache_policy().label(),
                cache_age_ms: resource.resource_cache_age_ms(now_ms),
                cache_hits: resource.resource_cache_hits(),
                retry_count: resource.resource_retry_count(),
            });
        }
    }

    /// Key/status pairs without the per-entry allocations of full
    /// diagnostics; used by `dehydrate`.
    #[cfg(feature = "persist")]
    pub(crate) fn collect_key_status_into(&self, cx: &App, out: &mut Vec<(String, QueryStatus)>) {
        for (key, entry) in self.entries.iter() {
            let Some(entity) = entry.entity.upgrade() else {
                continue;
            };
            let resource = entity.read(cx);
            out.push((key.to_path(), resource.resource_status()));
        }
    }

    /// Filter, max-age, and the caller's last-flushed data epochs all run
    /// before the serializer so skipped entries cost nothing. Only `Success`
    /// entries are collected; entries whose epoch matches `flushed` are
    /// reported as reused paths instead of re-serialized.
    #[cfg(feature = "persist")]
    pub(crate) fn collect_persistable_into<S>(
        &self,
        cx: &App,
        collect: &PersistCollect<'_>,
        out: &mut PersistCollectOut,
        value_of: impl Fn(&R) -> Option<&S>,
    ) where
        S: 'static,
    {
        use crate::core::QueryStatus;

        // Serializers are registered by `T` alone, not the `(T, E)` pair.
        let Some(serialize_fn) = collect.serializers.get(std::any::TypeId::of::<S>()) else {
            return;
        };
        for (key, entry) in self.entries.iter() {
            if !collect.filter.matches(key) {
                continue;
            }
            let Some(entity) = entry.entity.upgrade() else {
                continue;
            };
            let resource = entity.read(cx);
            if resource.resource_status() != QueryStatus::Success {
                continue;
            }
            let cached_at = resource.resource_last_updated().unwrap_or(collect.now_ms);
            if collect.max_age_ms > 0
                && collect.now_ms.saturating_sub(cached_at) > collect.max_age_ms
            {
                continue;
            }
            let Some(value_ref) = value_of(resource) else {
                continue;
            };
            let path = key.to_path();
            // Epochs restart at 0 on a recreated entry, so identity rides
            // alongside the epoch in the flush gate.
            let identity = (entity.entity_id(), resource.resource_data_epoch());
            if collect.flushed.get(&path) == Some(&identity) {
                out.reused.push(path);
                continue;
            }
            // Downcast failure is unreachable by construction; skip rather than persist junk.
            let Some(value) = serialize_fn(value_ref as &dyn std::any::Any) else {
                continue;
            };
            out.fresh.push(PersistCollected {
                key: key.clone(),
                path,
                entity_id: identity.0,
                epoch: identity.1,
                entry: PersistedEntry {
                    value,
                    cached_at,
                    cache_policy: resource.resource_cache_policy(),
                    meta: None,
                },
            });
        }
    }
}

/// One live persistable entry produced by
/// [`ResourceBucket::collect_persistable_into`].
#[cfg(feature = "persist")]
pub(crate) struct PersistCollected {
    pub(crate) key: QueryKey,
    pub(crate) path: String,
    pub(crate) entity_id: gpui::EntityId,
    pub(crate) epoch: u64,
    pub(crate) entry: PersistedEntry,
}

/// Per-sweep output: freshly serialized entries plus the paths of live
/// entries reused from the persist driver's store.
#[cfg(feature = "persist")]
#[derive(Default)]
pub(crate) struct PersistCollectOut {
    pub(crate) fresh: Vec<PersistCollected>,
    pub(crate) reused: Vec<String>,
}

/// Per-sweep inputs for [`ResourceBucket::collect_persistable_into`];
/// `flushed` maps paths to the owning entity id and data epoch at their
/// last flush.
#[cfg(feature = "persist")]
pub(crate) struct PersistCollect<'a> {
    serializers: &'a SerializerRegistry,
    filter: &'a PersistFilter,
    now_ms: u64,
    max_age_ms: u64,
    flushed: &'a std::collections::HashMap<String, (gpui::EntityId, u64)>,
}

#[cfg(feature = "persist")]
impl<'a> PersistCollect<'a> {
    pub(crate) fn new(
        serializers: &'a SerializerRegistry,
        filter: &'a PersistFilter,
        now_ms: u64,
        max_age_ms: u64,
        flushed: &'a std::collections::HashMap<String, (gpui::EntityId, u64)>,
    ) -> Self {
        Self {
            serializers,
            filter,
            now_ms,
            max_age_ms,
            flushed,
        }
    }
}

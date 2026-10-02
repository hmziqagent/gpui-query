//! Shares its machinery with [`QueryBucket`] through
//! [`ResourceBucket`](super::bucket::shared::ResourceBucket); only the first-page
//! persistence hook is infinite-specific.

use gpui::{App, Entity};

use crate::core::{
    CachePolicy, InfiniteQueryResource, QueryKey, QueryKeyFilter, RequestPolicy, RequestSequencer,
};

use super::bucket::shared::ResourceBucket;
use super::devtools::QueryDiagnostic;
use super::erased::ErasedBucket;

pub struct InfiniteQueryBucket<T, E> {
    entries: ResourceBucket<InfiniteQueryResource<T, E>>,
}

impl<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static> InfiniteQueryBucket<T, E> {
    pub(crate) fn new() -> Self {
        Self {
            entries: ResourceBucket::new(),
        }
    }

    pub(crate) fn get_or_create(
        &mut self,
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> Entity<InfiniteQueryResource<T, E>> {
        self.entries
            .get_or_create(key, cache_policy, request_policy, cx)
    }

    pub(crate) fn get(&self, key: &QueryKey) -> Option<Entity<InfiniteQueryResource<T, E>>> {
        self.entries.get(key)
    }

    pub(crate) fn sequencer_mut(&mut self, key: &QueryKey) -> Option<&mut RequestSequencer> {
        self.entries.sequencer_mut(key)
    }

    pub(crate) fn all_entities(&self) -> Vec<Entity<InfiniteQueryResource<T, E>>> {
        self.entries.all_entities()
    }
}

impl<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static> ErasedBucket
    for InfiniteQueryBucket<T, E>
{
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn gc(&mut self, now_ms: u64, gc_time_ms: u64, cx: &App) {
        self.entries.gc(now_ms, gc_time_ms, cx);
    }

    fn count(&self) -> usize {
        self.entries.entries.len()
    }

    fn invalidate_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.entries.invalidate_matching(filter, cx);
    }

    fn reset_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.entries.reset_matching(filter, cx);
    }

    fn remove_matching(&mut self, filter: &QueryKeyFilter) {
        self.entries.remove_matching(filter);
    }

    fn cancel_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.entries.cancel_matching(filter, cx);
    }

    fn collect_diagnostics_into(&self, now_ms: u64, cx: &App, out: &mut Vec<QueryDiagnostic>) {
        self.entries.collect_diagnostics_into(now_ms, cx, out);
    }

    #[cfg(feature = "persist")]
    fn collect_key_status_into(&self, cx: &App, out: &mut Vec<(String, crate::core::QueryStatus)>) {
        self.entries.collect_key_status_into(cx, out);
    }

    #[cfg(feature = "persist")]
    fn contains_key(&self, key: &crate::core::QueryKey) -> bool {
        self.entries.entries.contains_key(key)
    }

    /// Persists the first page only; the full page vector is opaque here.
    #[cfg(feature = "persist")]
    fn collect_persistable_into(
        &self,
        cx: &App,
        collect: &super::bucket::shared::PersistCollect<'_>,
        out: &mut super::bucket::shared::PersistCollectOut,
    ) {
        self.entries
            .collect_persistable_into(cx, collect, out, |r| r.first_page());
    }
}

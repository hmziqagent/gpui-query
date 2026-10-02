use gpui::App;

use crate::client::devtools::QueryDiagnostic;
use crate::client::erased::ErasedBucket;
use crate::core::QueryKeyFilter;

use super::ops::QueryBucket;

impl<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static> ErasedBucket
    for QueryBucket<T, E>
{
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn gc(&mut self, now_ms: u64, gc_time_ms: u64, cx: &App) {
        self.inner.gc(now_ms, gc_time_ms, cx);
    }

    fn count(&self) -> usize {
        self.inner.entries.len()
    }

    fn invalidate_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.inner.invalidate_matching(filter, cx);
    }

    fn reset_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.inner.reset_matching(filter, cx);
    }

    fn remove_matching(&mut self, filter: &QueryKeyFilter) {
        self.inner.remove_matching(filter);
    }

    fn cancel_matching(&mut self, filter: &QueryKeyFilter, cx: &mut App) {
        self.inner.cancel_matching(filter, cx);
    }

    fn collect_diagnostics_into(&self, now_ms: u64, cx: &App, out: &mut Vec<QueryDiagnostic>) {
        self.inner.collect_diagnostics_into(now_ms, cx, out);
    }

    #[cfg(feature = "persist")]
    fn collect_key_status_into(&self, cx: &App, out: &mut Vec<(String, crate::core::QueryStatus)>) {
        self.inner.collect_key_status_into(cx, out);
    }

    #[cfg(feature = "persist")]
    fn contains_key(&self, key: &crate::core::QueryKey) -> bool {
        self.inner.entries.contains_key(key)
    }

    #[cfg(feature = "persist")]
    fn collect_persistable_into(
        &self,
        cx: &App,
        collect: &crate::client::bucket::shared::PersistCollect<'_>,
        out: &mut crate::client::bucket::shared::PersistCollectOut,
    ) {
        self.inner
            .collect_persistable_into(cx, collect, out, |r| r.data());
    }
}

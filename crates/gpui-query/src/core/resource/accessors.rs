use crate::core::{
    CachePolicy, QueryKey, QuerySignal, QueryStatus, QueryTimestamp, RequestId, RequestPolicy,
    RetryPolicy,
};

use super::QueryResource;

impl<T, E> QueryResource<T, E> {
    pub fn is_loading(&self) -> bool {
        self.status.is_loading()
    }

    pub fn is_pending(&self) -> bool {
        self.status.is_pending()
    }

    pub fn key(&self) -> &QueryKey {
        &self.key
    }

    pub fn status(&self) -> QueryStatus {
        self.status
    }

    pub fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    pub fn error(&self) -> Option<&E> {
        self.error.as_ref()
    }

    pub fn active_request_id(&self) -> Option<RequestId> {
        self.active_request_id
    }

    pub fn cache_policy(&self) -> CachePolicy {
        self.cache_policy
    }

    pub fn request_policy(&self) -> RequestPolicy {
        self.request_policy
    }

    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at.map(QueryTimestamp::as_millis)
    }

    pub fn last_updated_at_ms(&self) -> Option<u64> {
        self.last_updated_at.map(QueryTimestamp::as_millis)
    }

    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    pub fn cancelled_count(&self) -> u64 {
        self.cancelled_count
    }

    pub fn ignored_results(&self) -> u64 {
        self.ignored_results
    }

    pub fn has_data(&self) -> bool {
        self.data.is_some()
    }

    /// Counts data writes, not value changes; compare instead of deep-comparing `T`.
    pub fn data_epoch(&self) -> u64 {
        self.data_epoch
    }

    pub fn signal(&self) -> Option<&QuerySignal> {
        self.signal.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn signal_mut(&mut self) -> Option<&mut QuerySignal> {
        self.signal.as_mut()
    }

    /// Saved during optimistic updates for [`rollback_to_previous`](Self::rollback_to_previous).
    pub fn previous_data(&self) -> Option<&T> {
        self.previous_data.as_ref()
    }

    pub fn retry_count(&self) -> u32 {
        self.retry_count
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }

    pub fn increment_retry(&mut self) {
        self.retry_count = self.retry_count.saturating_add(1);
    }

    pub fn set_retry_policy(&mut self, policy: RetryPolicy) {
        self.retry_policy = policy;
    }

    pub fn reset_retry_count(&mut self) {
        self.retry_count = 0;
    }

    pub fn set_cache_policy(&mut self, policy: CachePolicy) {
        self.cache_policy = policy;
    }

    pub fn set_request_policy(&mut self, policy: RequestPolicy) {
        self.request_policy = policy;
    }
}

use crate::core::{CachePolicy, QueryStatus, QueryTimestamp, RequestGuard, RequestId};

use super::QueryResource;

impl<T, E> QueryResource<T, E> {
    pub fn complete_current_success(
        &mut self,
        request_id: RequestId,
        data: T,
        now_ms: u64,
    ) -> bool {
        let Some(guard) = self.accept_current_request(request_id) else {
            return false;
        };
        self.complete_success(guard, data, now_ms);
        true
    }

    pub fn complete_current_failure(
        &mut self,
        request_id: RequestId,
        error: impl Into<E>,
        now_ms: u64,
    ) -> bool {
        let Some(guard) = self.accept_current_request(request_id) else {
            return false;
        };
        self.complete_failure(guard, error, now_ms);
        true
    }

    pub fn complete_current_optional_success(
        &mut self,
        request_id: RequestId,
        data: Option<T>,
        now_ms: u64,
    ) -> bool {
        let Some(guard) = self.accept_current_request(request_id) else {
            return false;
        };
        self.complete_success_optional(guard, data, now_ms);
        true
    }

    pub fn complete_current_failure_with_data(
        &mut self,
        request_id: RequestId,
        data: T,
        error: impl Into<E>,
        now_ms: u64,
    ) -> bool {
        let Some(guard) = self.accept_current_request(request_id) else {
            return false;
        };
        self.complete_failure_with_data(guard, data, error, now_ms);
        true
    }

    pub fn complete_success(&mut self, guard: RequestGuard, data: T, now_ms: u64) {
        self.validate_guard(&guard);
        self.apply_success(data, now_ms);
    }

    pub fn complete_failure(&mut self, guard: RequestGuard, error: impl Into<E>, now_ms: u64) {
        self.validate_guard(&guard);
        self.apply_failure(error, now_ms);
    }

    /// `None` data completes to [`QueryStatus::Idle`], not `Success`, keeping
    /// the invariant that `Success` implies data exists.
    pub fn complete_success_optional(&mut self, guard: RequestGuard, data: Option<T>, now_ms: u64) {
        self.validate_guard(&guard);
        self.apply_success_optional(data, now_ms);
    }

    pub fn complete_failure_with_data(
        &mut self,
        guard: RequestGuard,
        data: T,
        error: impl Into<E>,
        now_ms: u64,
    ) {
        self.validate_guard(&guard);
        self.apply_failure_with_data(data, error, now_ms);
    }

    pub(crate) fn apply_success(&mut self, data: T, now_ms: u64) {
        self.previous_data = self.data.take();
        self.status = QueryStatus::Success;
        self.data = Some(data);
        self.error = None;
        self.active_request_id = None;
        self.last_updated_at = Some(QueryTimestamp::from(now_ms));
        self.data_epoch = self.data_epoch.saturating_add(1);
    }

    pub(crate) fn apply_failure(&mut self, error: impl Into<E>, now_ms: u64) {
        self.status = QueryStatus::Failure;
        self.error = Some(error.into());
        self.active_request_id = None;
        self.last_updated_at = Some(QueryTimestamp::from(now_ms));
    }

    pub(crate) fn apply_success_optional(&mut self, data: Option<T>, now_ms: u64) {
        self.previous_data = self.data.take();
        if data.is_some() {
            self.status = QueryStatus::Success;
        } else {
            self.status = QueryStatus::Idle;
        }
        self.data = data;
        self.error = None;
        self.active_request_id = None;
        self.last_updated_at = Some(QueryTimestamp::from(now_ms));
        self.data_epoch = self.data_epoch.saturating_add(1);
    }

    pub(crate) fn apply_failure_with_data(&mut self, data: T, error: impl Into<E>, now_ms: u64) {
        self.status = QueryStatus::Failure;
        self.data = Some(data);
        self.error = Some(error.into());
        self.active_request_id = None;
        self.last_updated_at = Some(QueryTimestamp::from(now_ms));
        self.data_epoch = self.data_epoch.saturating_add(1);
    }

    /// `accept_current_request` clears `active_request_id`, so a `Some` here
    /// means a `begin_request` was interleaved between accept and complete.
    fn validate_guard(&self, guard: &RequestGuard) {
        debug_assert!(
            self.active_request_id.is_none(),
            "complete_* called with stale guard (id={}) but a new request is active ({:?}). \
             The caller must not interleave begin_request between accept and complete.",
            guard.request_id().label(),
            self.active_request_id,
        );
    }
}

impl<T, E> QueryResource<T, E> {
    /// `NoCache` data can never produce a cache hit, so clear it after
    /// observers consume it instead of holding it in memory.
    pub fn should_clear_data_on_complete(&self) -> bool {
        matches!(self.cache_policy, CachePolicy::NoCache)
    }
}

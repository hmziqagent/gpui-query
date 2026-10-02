use serde::{Deserialize, Serialize};

use super::{
    CachePolicy, QueryError, QueryKey, QuerySignal, QueryStatus, QueryTimestamp, RequestId,
    RequestPolicy, RequestSequencer, RetryPolicy,
};

mod accessors;
mod cache;
mod completion;
mod lifecycle;

/// Framework-free state machine for one query: data, error, status, retries,
/// and a cooperative cancellation signal. Lifecycle: `begin_request` →
/// `accept_current_request` → `complete_*`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResource<T, E = QueryError> {
    key: QueryKey,
    status: QueryStatus,
    data: Option<T>,
    error: Option<E>,
    active_request_id: Option<RequestId>,
    cache_policy: CachePolicy,
    request_policy: RequestPolicy,
    started_at: Option<QueryTimestamp>,
    last_updated_at: Option<QueryTimestamp>,
    cache_hits: u64,
    cancelled_count: u64,
    ignored_results: u64,
    retry_count: u32,
    retry_policy: RetryPolicy,
    previous_data: Option<T>,
    /// Counts data writes, not value changes; runtime only, so change
    /// detection never deep-compares `T`.
    #[serde(skip)]
    data_epoch: u64,
    /// Runtime state, not persisted; supplies monotonic ids when no external
    /// sequencer is provided.
    #[serde(skip)]
    transient_sequencer: RequestSequencer,
    #[serde(skip)]
    signal: Option<QuerySignal>,
}

/// Observable state only; `data_epoch` is bookkeeping and must not split two
/// otherwise-equal resources.
impl<T: PartialEq, E: PartialEq> PartialEq for QueryResource<T, E> {
    fn eq(&self, other: &Self) -> bool {
        let QueryResource {
            key,
            status,
            data,
            error,
            active_request_id,
            cache_policy,
            request_policy,
            started_at,
            last_updated_at,
            cache_hits,
            cancelled_count,
            ignored_results,
            retry_count,
            retry_policy,
            previous_data,
            data_epoch: _,
            transient_sequencer,
            signal,
        } = self;
        *key == other.key
            && *status == other.status
            && *data == other.data
            && *error == other.error
            && *active_request_id == other.active_request_id
            && *cache_policy == other.cache_policy
            && *request_policy == other.request_policy
            && *started_at == other.started_at
            && *last_updated_at == other.last_updated_at
            && *cache_hits == other.cache_hits
            && *cancelled_count == other.cancelled_count
            && *ignored_results == other.ignored_results
            && *retry_count == other.retry_count
            && *retry_policy == other.retry_policy
            && *previous_data == other.previous_data
            && *transient_sequencer == other.transient_sequencer
            && *signal == other.signal
    }
}

impl<T: PartialEq + Eq, E: PartialEq + Eq> Eq for QueryResource<T, E> {}

impl<T, E> QueryResource<T, E> {
    pub fn new(
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
    ) -> Self {
        Self {
            key: key.into(),
            status: QueryStatus::Idle,
            data: None,
            error: None,
            active_request_id: None,
            cache_policy,
            request_policy,
            started_at: None,
            last_updated_at: None,
            cache_hits: 0,
            cancelled_count: 0,
            ignored_results: 0,
            retry_count: 0,
            retry_policy: RetryPolicy::no_retries(),
            previous_data: None,
            data_epoch: 0,
            transient_sequencer: RequestSequencer::new(),
            signal: None,
        }
    }
}

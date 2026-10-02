use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::core::{
    CachePolicy, QueryError, QueryKey, QuerySignal, QueryStatus, QueryTimestamp, RequestId,
    RequestPolicy, RequestSequencer, RetryPolicy,
};

const DEFAULT_MAX_PAGES: usize = 50;

/// `ForwardOnly` (default) starts `has_next_page = true`; `Bidirectional`
/// starts both flags `false`, so nothing is fetched until the caller or
/// fetcher opts in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchDirection {
    #[default]
    ForwardOnly,
    Bidirectional,
}

/// Pages are stored as `Arc<T>` so the `*_arc` accessors can hand the fetcher
/// a cheap clone instead of copying the page.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound(serialize = "T: serde::Serialize, E: serde::Serialize"))]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned, E: serde::de::DeserializeOwned"))]
pub struct InfiniteQueryResource<T, E = QueryError> {
    pub(super) key: QueryKey,
    #[serde(with = "vec_deque_serde")]
    pub(super) pages: VecDeque<Arc<T>>,
    pub(super) status: QueryStatus,
    pub(super) error: Option<E>,
    pub(super) active_request_id: Option<RequestId>,
    pub(super) cache_policy: CachePolicy,
    pub(super) request_policy: RequestPolicy,
    pub(super) started_at: Option<QueryTimestamp>,
    pub(super) last_updated_at: Option<QueryTimestamp>,
    /// Counts page writes, not value changes; runtime only, so change
    /// detection never deep-compares `T`.
    #[serde(skip)]
    pub(super) data_epoch: u64,
    pub(super) cache_hits: u64,
    pub(super) cancelled_count: u64,
    pub(super) ignored_results: u64,
    pub(super) retry_count: u32,
    pub(super) has_next_page: bool,
    pub(super) has_previous_page: bool,
    /// One `Option` instead of two booleans: "only one direction in flight"
    /// holds by construction.
    pub(super) fetching_direction: Option<super::lifecycle::PageDirection>,
    pub(super) max_pages: Option<usize>,
    pub(super) direction: FetchDirection,
    pub(super) retry_policy: RetryPolicy,
    /// Runtime state, not persisted; supplies monotonic ids when no external
    /// sequencer is provided.
    #[serde(skip)]
    pub(super) transient_sequencer: RequestSequencer,
    #[serde(skip)]
    pub(super) signal: Option<QuerySignal>,
    #[cfg(feature = "client")]
    #[serde(skip)]
    pub(crate) current_task: crate::core::current_task::CurrentTask,
}

/// Observable state only; `data_epoch` is bookkeeping and must not split two
/// otherwise-equal resources, matching `QueryResource`.
impl<T: PartialEq, E: PartialEq> PartialEq for InfiniteQueryResource<T, E> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.pages == other.pages
            && self.status == other.status
            && self.error == other.error
            && self.active_request_id == other.active_request_id
            && self.cache_policy == other.cache_policy
            && self.request_policy == other.request_policy
            && self.started_at == other.started_at
            && self.last_updated_at == other.last_updated_at
            && self.cache_hits == other.cache_hits
            && self.cancelled_count == other.cancelled_count
            && self.ignored_results == other.ignored_results
            && self.retry_count == other.retry_count
            && self.has_next_page == other.has_next_page
            && self.has_previous_page == other.has_previous_page
            && self.fetching_direction == other.fetching_direction
            && self.max_pages == other.max_pages
            && self.direction == other.direction
            && self.retry_policy == other.retry_policy
            && self.transient_sequencer == other.transient_sequencer
            && self.signal == other.signal
            && self.current_task_eq(other)
    }
}

impl<T: PartialEq + Eq, E: PartialEq + Eq> Eq for InfiniteQueryResource<T, E> {}

impl<T, E> InfiniteQueryResource<T, E> {
    #[cfg(feature = "client")]
    fn current_task_eq(&self, other: &Self) -> bool {
        self.current_task == other.current_task
    }

    #[cfg(not(feature = "client"))]
    fn current_task_eq(&self, _other: &Self) -> bool {
        true
    }
}

/// The wire format is a plain sequence, identical to the old `Vec<T>`
/// representation, so previously persisted data stays readable.
pub(super) mod vec_deque_serde {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use serde::de::{Deserialize, DeserializeOwned};
    use serde::ser::SerializeSeq;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S, T>(deque: &VecDeque<Arc<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: serde::Serialize,
    {
        let mut seq = serializer.serialize_seq(Some(deque.len()))?;
        for item in deque {
            // Serialize the inner T: requiring `Arc<T>: Serialize` would need serde's `rc` feature.
            seq.serialize_element(&**item)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<VecDeque<Arc<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let vec: Vec<T> = Vec::<T>::deserialize(deserializer)?;
        Ok(vec.into_iter().map(Arc::new).collect())
    }
}

impl<T, E> InfiniteQueryResource<T, E> {
    /// `max_pages` defaults to `Some(50)`; ForwardOnly, so `has_next_page`
    /// starts `true`.
    pub fn new(
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
    ) -> Self {
        Self::with_direction(
            key,
            cache_policy,
            request_policy,
            FetchDirection::ForwardOnly,
        )
    }

    /// Both flags start `false`; the query fetches nothing until the caller
    /// explicitly enables a direction.
    pub fn new_bidirectional(
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
    ) -> Self {
        Self::with_direction(
            key,
            cache_policy,
            request_policy,
            FetchDirection::Bidirectional,
        )
    }

    pub(crate) fn with_direction(
        key: impl Into<QueryKey>,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        direction: FetchDirection,
    ) -> Self {
        let (has_next, has_prev) = match direction {
            FetchDirection::ForwardOnly => (true, false),
            FetchDirection::Bidirectional => (false, false),
        };
        Self {
            key: key.into(),
            pages: VecDeque::new(),
            status: QueryStatus::Idle,
            error: None,
            active_request_id: None,
            cache_policy,
            request_policy,
            started_at: None,
            last_updated_at: None,
            data_epoch: 0,
            cache_hits: 0,
            cancelled_count: 0,
            ignored_results: 0,
            retry_count: 0,
            has_next_page: has_next,
            has_previous_page: has_prev,
            fetching_direction: None,
            max_pages: Some(DEFAULT_MAX_PAGES),
            direction,
            retry_policy: RetryPolicy::default(),
            transient_sequencer: RequestSequencer::new(),
            signal: None,
            #[cfg(feature = "client")]
            current_task: crate::core::current_task::CurrentTask::default(),
        }
    }
}

#[cfg(feature = "client")]
impl<T, E> InfiniteQueryResource<T, E> {
    pub(crate) fn set_current_task(&mut self, task: gpui::Task<()>) {
        self.current_task.set(task);
    }
}

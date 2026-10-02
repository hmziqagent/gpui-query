use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext as _, BackgroundExecutor, Entity, TestAppContext};

#[cfg(feature = "client")]
use crate::client::QueryClient;
use crate::core::{
    CachePolicy, QueryBeginResult, QueryFetchMode, QueryKey, QueryResource, QueryStatus, RequestId,
    RequestPolicy, RequestSequencer,
};
#[cfg(feature = "hook")]
use crate::hook::MutationOptions;

#[cfg(feature = "client")]
pub fn setup_query_client(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(QueryClient::new());
    });
}

#[cfg(feature = "client")]
pub fn setup_test(cx: &mut TestAppContext) {
    setup_query_client(cx);
}

#[cfg(feature = "client")]
pub fn setup_query_client_with_policies(
    cx: &mut TestAppContext,
    cache_policy: CachePolicy,
    request_policy: RequestPolicy,
) {
    cx.update(|cx| {
        cx.set_global(QueryClient::with_policies(cache_policy, request_policy));
    });
}

#[cfg(feature = "client")]
pub fn setup_query_client_with_gc(cx: &mut TestAppContext, gc_time_ms: u64) {
    cx.update(|cx| {
        cx.set_global(QueryClient::new().with_gc_time(gc_time_ms));
    });
}

pub fn test_resource() -> QueryResource<&'static str> {
    QueryResource::new(
        "test",
        CachePolicy::Ttl { ttl_ms: 1_000 },
        RequestPolicy::LatestWins,
    )
}

pub fn test_resource_with_policies(
    key: impl Into<QueryKey>,
    cache_policy: CachePolicy,
    request_policy: RequestPolicy,
) -> QueryResource<&'static str> {
    QueryResource::new(key, cache_policy, request_policy)
}

pub fn test_sequencer() -> RequestSequencer {
    RequestSequencer::new()
}

pub fn assert_status(resource: &QueryResource<impl Clone, impl Clone>, expected: QueryStatus) {
    let actual = resource.status();
    assert_eq!(
        actual, expected,
        "expected status {:?} but got {:?}",
        expected, actual
    );
}

pub fn nocache_resource(key: impl Into<QueryKey>) -> QueryResource<&'static str> {
    QueryResource::new(key, CachePolicy::NoCache, RequestPolicy::LatestWins)
}

pub fn begin_request_id(
    r: &mut QueryResource<impl Clone, impl Clone>,
    seq: &mut RequestSequencer,
    now_ms: u64,
    mode: QueryFetchMode,
) -> RequestId {
    match r.begin_request(seq, now_ms, mode) {
        QueryBeginResult::Started { request_id, .. } => request_id,
        other => panic!(
            "begin_request_id() expected Started, got {:?} \
             (status={:?}, active_request_id={:?})",
            other,
            r.status(),
            r.active_request_id(),
        ),
    }
}

#[cfg(feature = "hook")]
pub fn no_retry_mutation_options() -> MutationOptions {
    MutationOptions {
        retry_policy: crate::core::RetryPolicy::no_retries(),
        gc_time_ms: 300_000,
    }
}

#[derive(Default)]
pub(crate) struct DummyView;

#[cfg(feature = "client")]
pub fn observe_with_dummy_view<T, E>(
    cx: &mut App,
    observer: &mut crate::client::QueryObserver<T, E>,
) -> Option<gpui::Subscription>
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    let view: Entity<DummyView> = cx.new(|_| DummyView);
    view.update(cx, |_view, cx| observer.observe(cx))
}

pub struct HookHarness<T> {
    pub entity: Entity<T>,
}

impl<T> HookHarness<T> {
    pub fn new(entity: Entity<T>) -> Self {
        Self { entity }
    }
}

pub fn run_until_parked_and_read<T, R>(
    cx: &mut TestAppContext,
    entity: &Entity<T>,
    f: impl FnOnce(&T, &App) -> R,
) -> R
where
    T: 'static,
{
    cx.run_until_parked();
    cx.update(|cx| entity.read_with(cx, f))
}

pub struct Gate {
    inner: Arc<Mutex<bool>>,
}

impl Clone for Gate {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(false)),
        }
    }

    pub fn release(&self) {
        *self.inner.lock().unwrap() = true;
    }

    pub fn is_released(&self) -> bool {
        *self.inner.lock().unwrap()
    }

    pub async fn wait(&self, executor: &BackgroundExecutor) {
        while !self.is_released() {
            executor.timer(Duration::from_millis(1)).await;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct User {
    pub id: u32,
    pub name: String,
}

impl User {
    pub fn new(id: u32, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Post;

pub const TEST_NOW_MS: u64 = 1_000_000;

pub fn assert_serde_roundtrip<T>(cases: &[T])
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    for value in cases {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            &back, value,
            "serde roundtrip changed value (json={})",
            json
        );
    }
}

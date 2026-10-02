//! Request lifecycle helpers and retry-aware fetch functions for query resources.

use gpui::{BorrowAppContext as _, Context, Entity};

use crate::client::QueryClient;
use crate::core::{
    CachePolicy, Fetched, QueryBeginResult, QueryFetchMode, QueryKey, QueryResource, QuerySignal,
    RequestId, RetryPolicy,
};

use super::{current_time_ms, read_entity};

pub(crate) struct FetchParts<T> {
    pub data: T,
    pub server_policy: Option<CachePolicy>,
    #[cfg(feature = "persist")]
    pub meta: Option<serde_json::Value>,
}

/// Plain `T` yields no server policy; [`Fetched<T>`] carries the optional
/// policy and (under `persist`) meta.
pub(crate) trait FetchedLike<T> {
    fn into_parts(self) -> FetchParts<T>;
}

impl<T> FetchedLike<T> for T {
    fn into_parts(self) -> FetchParts<T> {
        FetchParts {
            data: self,
            server_policy: None,
            #[cfg(feature = "persist")]
            meta: None,
        }
    }
}

impl<T> FetchedLike<T> for Fetched<T> {
    fn into_parts(self) -> FetchParts<T> {
        FetchParts {
            data: self.data,
            server_policy: self.cache_policy,
            #[cfg(feature = "persist")]
            meta: self.meta,
        }
    }
}

/// Freshness check, `Loading` transition, and signal read in one
/// `entity.update`; `(None, None)` means no fetch: `CacheHit`,
/// `IgnoredWhileLoading`, or a revalidate already in flight.
///
/// With a [`QueryClient`], the bucket sequencer mints the `RequestId`, shared
/// with `prepare_fetch_query` so the two never collide for the same key.
pub(crate) fn begin_request_on_entity<T, E, C>(
    entity: &Entity<QueryResource<T, E>>,
    cx: &mut Context<C>,
    fetch_mode: QueryFetchMode,
    known_key: Option<QueryKey>,
) -> (Option<RequestId>, Option<QuerySignal>)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: 'static,
{
    let now_ms = current_time_ms();

    let maybe_request_id = if cx.has_global::<QueryClient>() {
        let key = known_key.unwrap_or_else(|| entity.read_with(cx, |r, _| r.key().clone()));
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client.next_request_id_for_key::<T, E>(&key)
        })
    } else {
        None
    };

    entity.update(cx, |resource, _cx| {
        let active_before = resource.active_request_id();
        match resource.begin_request_with_id(maybe_request_id, now_ms, fetch_mode) {
            // IgnoreWhileLoading hands back the still-active id unminted;
            // spawning here would race the original fetcher.
            QueryBeginResult::StaleCacheHit {
                request_id,
                replaced_request_id: None,
                ..
            } if Some(request_id) == active_before => (None, None),
            QueryBeginResult::Started { request_id, .. }
            | QueryBeginResult::StaleCacheHit { request_id, .. } => {
                let signal = resource.signal().cloned();
                (Some(request_id), signal)
            }
            QueryBeginResult::CacheHit | QueryBeginResult::IgnoredWhileLoading { .. } => {
                (None, None)
            }
        }
    })
}

/// Shared retry loop; with `signal = Some`, a fresh signal is re-read after
/// each delay, and the loop stops once a newer request supersedes this one.
/// `cx.notify()` fires only on accepted results; retry counters stay in
/// `Loading` (the observer dedupes on status or data epoch).
async fn run_query_retry_loop<T, E, Out, F, Fut>(
    fetcher: F,
    request_id: RequestId,
    retry_policy: &RetryPolicy,
    entity: &gpui::WeakEntity<QueryResource<T, E>>,
    cx: &mut gpui::AsyncApp,
    mut signal: Option<QuerySignal>,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    Out: FetchedLike<T> + Send + 'static,
    F: Fn(Option<QuerySignal>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Out, E>> + Send + 'static,
{
    let mut attempt: u32 = 0;

    loop {
        let result = fetcher(signal.clone()).await;

        match result {
            Ok(out) => {
                let parts = out.into_parts();
                let now_ms = current_time_ms();
                let Some(e) = entity.upgrade() else {
                    return;
                };
                e.update(cx, |resource, cx| {
                    if let Some(guard) = resource.accept_current_request(request_id) {
                        resource.reset_retry_count();
                        resource.complete_success(guard, parts.data, now_ms);
                        if let Some(policy) = parts.server_policy {
                            resource.set_cache_policy(policy);
                        }
                        cx.notify();
                        #[cfg(feature = "persist")]
                        cx.default_global::<crate::client::CacheMutation>();
                        #[cfg(feature = "persist")]
                        if let Some(meta) = parts.meta {
                            let key = resource.key().clone();
                            cx.update_global::<QueryClient, _>(|client, _| {
                                client.record_meta(key, meta);
                            });
                        }
                    }
                });
                return;
            }
            Err(error) => {
                if retry_policy.should_retry(attempt) {
                    let delay_ms = retry_policy.delay_for_attempt(attempt);
                    attempt += 1;

                    if delay_ms > 0 {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(delay_ms))
                            .await;
                    }

                    let Some(e) = entity.upgrade() else { return };
                    let (request_still_active, fresh_signal) = read_entity(&e, cx, |r, _| {
                        (
                            r.is_current_request(request_id),
                            r.signal().cloned().unwrap_or_else(QuerySignal::new),
                        )
                    })
                    .unwrap_or_else(|| (false, QuerySignal::new()));
                    if !request_still_active {
                        return;
                    }
                    e.update(cx, |resource, _cx| {
                        resource.increment_retry();
                    });
                    if let Some(ref mut sig) = signal {
                        *sig = fresh_signal;
                    }
                } else {
                    let Some(e) = entity.upgrade() else { return };
                    let failure_now_ms = current_time_ms();
                    e.update(cx, |resource, cx| {
                        if let Some(guard) = resource.accept_current_request(request_id) {
                            resource.reset_retry_count();
                            resource.complete_failure(guard, error, failure_now_ms);
                            cx.notify();
                            #[cfg(feature = "persist")]
                            cx.default_global::<crate::client::CacheMutation>();
                        }
                    });
                    return;
                }
            }
        }
    }
}

/// No-signal wrapper over [`run_query_retry_loop`].
pub(crate) async fn fetch_with_retry<T, E, Out, F, Fut>(
    fetcher: F,
    request_id: RequestId,
    retry_policy: &RetryPolicy,
    entity: &gpui::WeakEntity<QueryResource<T, E>>,
    cx: &mut gpui::AsyncApp,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    Out: FetchedLike<T> + Send + 'static,
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Out, E>> + Send + 'static,
{
    let wrapper = move |_: Option<QuerySignal>| fetcher();
    run_query_retry_loop::<T, E, Out, _, _>(wrapper, request_id, retry_policy, entity, cx, None)
        .await;
}

/// Signal variant: each retry hands the fetcher a fresh signal from the resource.
pub(crate) async fn fetch_signal_with_retry<T, E, Out, F, Fut>(
    fetcher: F,
    initial_signal: QuerySignal,
    request_id: RequestId,
    retry_policy: &RetryPolicy,
    entity: &gpui::WeakEntity<QueryResource<T, E>>,
    cx: &mut gpui::AsyncApp,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    Out: FetchedLike<T> + Send + 'static,
    F: Fn(QuerySignal) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Out, E>> + Send + 'static,
{
    let wrapper = move |sig: Option<QuerySignal>| fetcher(sig.unwrap_or_default());
    run_query_retry_loop::<T, E, Out, _, _>(
        wrapper,
        request_id,
        retry_policy,
        entity,
        cx,
        Some(initial_signal),
    )
    .await;
}

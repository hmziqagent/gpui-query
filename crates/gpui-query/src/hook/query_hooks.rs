//! Fetch tasks are deliberately detached: stale writes are guarded by the
//! two-phase `accept_current_request` protocol, and each task holds only a
//! `WeakEntity`, so it self-terminates on entity drop.

use gpui::{BorrowAppContext as _, Context, Entity, Subscription};
// Only the release-profile fallback below calls `AppContext::new`; importing
// it unconditionally warns as unused in dev builds, hence the cfg gate.
#[cfg(not(debug_assertions))]
use gpui::AppContext as _;

use crate::client::{QueryClient, QueryObserver};
use crate::core::{
    CachePolicy, Fetched, QueryFetchMode, QueryKey, QueryResource, QuerySignal, QueryStatus,
    RequestPolicy,
};

use super::QueryOptions;
use super::fetch_retry::{
    FetchedLike, begin_request_on_entity, fetch_signal_with_retry, fetch_with_retry,
};

/// Creates or reuses the resource in the global [`QueryClient`] and spawns a
/// fetch if it is idle; call it in a constructor, never in `render`.
///
/// # Example
///
/// ```no_run
/// use gpui_query::hook::use_query;
/// use gpui_query::{QueryOptions, CachePolicy, RequestPolicy};
/// # #[derive(Clone)]
/// # struct User;
/// # #[derive(Clone, Debug)]
/// # struct MyError;
///
/// struct MyView {
///     users: gpui::Entity<gpui_query::QueryResource<Vec<User>, MyError>>,
///     _subscription: gpui::Subscription,
/// }
///
/// impl MyView {
///     fn new(cx: &mut gpui::Context<Self>) -> Self {
///         let (users, _subscription) = use_query(
///             QueryOptions::new("users")
///                 .cache_policy(CachePolicy::Ttl { ttl_ms: 60_000 })
///                 .request_policy(RequestPolicy::LatestWins),
///             |signal| async move {
///                 Ok(vec![])
///             },
///             cx,
///         );
///         Self { users, _subscription }
///     }
/// }
/// ```
pub fn use_query<T, E, C, F, Fut>(
    options: impl Into<QueryOptions>,
    fetcher: F,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(QuerySignal) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    use_query_impl(options.into(), fetcher, cx)
}

/// A fetcher returning [`Fetched::with_policy`](crate::core::Fetched::with_policy)
/// overrides the resource's stored policy right after success (server wins).
pub fn use_query_with_policy<T, E, C, F, Fut>(
    options: impl Into<QueryOptions>,
    fetcher: F,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(QuerySignal) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Fetched<T>, E>> + Send + 'static,
{
    use_query_impl(options.into(), fetcher, cx)
}

fn use_query_impl<T, E, C, F, Fut, Out>(
    options: QueryOptions,
    fetcher: F,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    Out: FetchedLike<T> + Send + 'static,
    F: Fn(QuerySignal) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Out, E>> + Send + 'static,
{
    let QueryOptions {
        key,
        cache_policy,
        request_policy,
        retry_policy,
        force_fetch,
        ..
    } = options;
    let (entity, subscription) = use_query_manual(key.clone(), cache_policy, request_policy, cx);

    entity.update(cx, |r, _| r.set_retry_policy(retry_policy.clone()));

    if entity.read_with(cx, |r, _| r.status() == QueryStatus::Idle) {
        let fetch_mode = if force_fetch {
            QueryFetchMode::Force
        } else {
            QueryFetchMode::Normal
        };
        if let (Some(request_id), signal) =
            begin_request_on_entity(&entity, cx, fetch_mode, Some(key))
        {
            let signal = signal.unwrap_or_else(QuerySignal::new);
            let weak = entity.downgrade();
            cx.spawn(async move |_this, cx| {
                fetch_signal_with_retry(fetcher, signal, request_id, &retry_policy, &weak, cx)
                    .await;
            })
            .detach();
        }
    }

    (entity, subscription)
}

/// Signal-free fetcher variant; prefer the signal-accepting [`use_query`].
pub fn use_query_unsignalled<T, E, C, F, Fut>(
    key: QueryKey,
    cache_policy: CachePolicy,
    request_policy: RequestPolicy,
    fetcher: F,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let (entity, subscription) = use_query_manual(key, cache_policy, request_policy, cx);

    if entity.read_with(cx, |r, _| r.status() == QueryStatus::Idle) {
        spawn_retry_fetch(&entity, fetcher, cx);
    }

    (entity, subscription)
}

/// Consumes only `key`, `cache_policy`, and `request_policy`; use
/// [`use_query`] to honor the rest.
pub fn use_query_manual_opts<T, E, C>(
    options: impl Into<QueryOptions>,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: 'static,
{
    let opts = options.into();
    use_query_manual(opts.key, opts.cache_policy, opts.request_policy, cx)
}

pub fn use_query_unsignalled_opts<T, E, C, F, Fut>(
    options: impl Into<QueryOptions>,
    fetcher: F,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let opts = options.into();
    use_query_unsignalled(
        opts.key,
        opts.cache_policy,
        opts.request_policy,
        fetcher,
        cx,
    )
}

/// Entity + observer without starting a fetch; panics in debug builds when no [`QueryClient`] global is set (release falls back to a standalone entity).
pub fn use_query_manual<T, E, C>(
    key: QueryKey,
    cache_policy: CachePolicy,
    request_policy: RequestPolicy,
    cx: &mut Context<C>,
) -> (Entity<QueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: 'static,
{
    let entity = if cx.has_global::<QueryClient>() {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.resource_with_policies::<T, E>(key, cache_policy, request_policy, cx)
        })
    } else {
        #[cfg(debug_assertions)]
        {
            panic!(
                "use_query_manual: QueryClient is not initialized. \
                 Call cx.set_global(QueryClient::new()) before using query hooks."
            );
        }
        #[cfg(not(debug_assertions))]
        {
            cx.new(|_| QueryResource::new(key, cache_policy, request_policy))
        }
    };

    let observer = QueryObserver::new(&entity);
    let Some(subscription) = observer.observe(cx) else {
        debug_assert!(
            false,
            "QueryObserver::observe failed: entity was just created and cannot be dropped"
        );
        return (entity, Subscription::new(|| {}));
    };

    (entity, subscription)
}

/// No-op when the cache is fresh or a fetch is already loading; otherwise
/// spawns a retry-aware fetch.
pub fn fetch_query<T, E, C, F, Fut>(
    entity: &Entity<QueryResource<T, E>>,
    fetcher: F,
    cx: &mut Context<C>,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    spawn_retry_fetch(entity, fetcher, cx);
}

/// [`fetch_query`] whose fetcher may return [`Fetched<T>`](crate::core::Fetched)
/// to override the resource's policy on success.
pub fn fetch_query_with_policy<T, E, C, F, Fut>(
    entity: &Entity<QueryResource<T, E>>,
    fetcher: F,
    cx: &mut Context<C>,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Fetched<T>, E>> + Send + 'static,
{
    spawn_retry_fetch(entity, fetcher, cx);
}

fn spawn_retry_fetch<T, E, C, F, Fut, Out>(
    entity: &Entity<QueryResource<T, E>>,
    fetcher: F,
    cx: &mut Context<C>,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    Out: FetchedLike<T> + Send + 'static,
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Out, E>> + Send + 'static,
{
    let (Some(request_id), _signal) =
        begin_request_on_entity(entity, cx, QueryFetchMode::Normal, None)
    else {
        return;
    };
    let weak = entity.downgrade();
    let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
    cx.spawn(async move |_this, cx| {
        fetch_with_retry(fetcher, request_id, &retry_policy, &weak, cx).await;
    })
    .detach();
}

/// `FnOnce` fetcher, so no retries; staleness is guarded by
/// `accept_current_request`, not a post-fetch `is_cancelled()` check (racy).
pub fn fetch_query_with_signal<T, E, C, F, Fut>(
    entity: &Entity<QueryResource<T, E>>,
    fetcher: F,
    cx: &mut Context<C>,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: FnOnce(QuerySignal) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let (Some(request_id), signal) =
        begin_request_on_entity(entity, cx, QueryFetchMode::Normal, None)
    else {
        return;
    };
    let signal = signal.unwrap_or_else(QuerySignal::new);
    let weak = entity.downgrade();

    cx.spawn(async move |_this, cx| {
        let result = fetcher(signal).await;

        let now_ms = super::current_time_ms();
        let Some(entity) = weak.upgrade() else { return };

        entity.update(cx, |resource, cx| {
            if let Some(guard) = resource.accept_current_request(request_id) {
                match result {
                    Ok(data) => {
                        resource.reset_retry_count();
                        resource.complete_success(guard, data, now_ms);
                    }
                    Err(error) => {
                        resource.reset_retry_count();
                        resource.complete_failure(guard, error, now_ms);
                    }
                }
                cx.notify();
            }
        });
    })
    .detach();
}

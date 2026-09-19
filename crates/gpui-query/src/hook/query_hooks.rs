//! Query hook functions — `use_query`, `use_query_unsignalled`, `use_query_manual`,
//! `fetch_query`, and `fetch_query_with_signal`.
//!
//! # Task lifecycle: deliberate detach-by-design (Audit Finding #6)
//!
//! Audit fix #6 (storing the spawned fetch task so a replacement fetch or
//! entity drop aborts it) is applied to **mutations and infinite queries**.
//! The **plain-query** spawn sites in this module — inside `use_query`,
//! `use_query_unsignalled`, `fetch_query`, and `fetch_query_with_signal` —
//! intentionally call `task.detach()` instead. This is not an unfinished fix:
//!
//! - Query fetches already prevent stale writes through the cooperative
//!   `QuerySignal` plus the `is_current_request` / `accept_current_request`
//!   two-phase guard in the retry loop. A superseded fetcher still observes
//!   its cancelled signal, which tests enforce.
//! - Hard-aborting a query task on replacement would break that cooperative
//!   contract. The detached task self-terminates once the owning entity is
//!   dropped (the `weak.upgrade()` checks return `None`), so it cannot leak
//!   writes after unmount.
//!
//! Each site repeats a short form of this rationale next to its `detach()`.

use gpui::{BorrowAppContext as _, Context, Entity, Subscription};
// Only the release-profile fallback below calls `AppContext::new`; importing
// it unconditionally warns as unused in dev builds, hence the cfg gate.
#[cfg(not(debug_assertions))]
use gpui::AppContext as _;

use crate::client::{QueryClient, QueryObserver};
use crate::core::{Fetched, QueryFetchMode, QueryKey, QueryResource, QuerySignal, QueryStatus};

use super::current_time_ms;
use super::fetch_retry::{begin_request_on_entity, fetch_signal_with_retry, fetch_with_retry};

/// Subscribe to a query resource and automatically re-render when it changes.
///
/// This is the **primary** `use_query` hook following the v2 "Signal-always"
/// design: the fetcher receives a [`QuerySignal`] for cooperative cancellation.
///
/// Call this in your component's constructor (not in `render`). It:
///
/// 1. Gets or creates a [`QueryResource`] entity from the global [`QueryClient`]
/// 2. Sets up a [`QueryObserver`] so your component re-renders on state changes
/// 3. Propagates the user's retry policy to the resource entity (audit fix #16)
/// 4. Calls `begin_request` to set status to Loading and obtain a `RequestId`
/// 5. Spawns an async fetch with retry logic, using the stored `RequestId`
///
/// # Returns
///
/// A tuple of `(Entity<QueryResource<T, E>>, Subscription)`:
/// - Store the entity to read state during render
/// - Store the subscription to keep the observation alive
///
/// # Unmount Behavior (Audit Finding #6)
///
/// If the component unmounts while a fetch is in-flight, the fetch result is
/// silently discarded. No callback fires. This is intentional for cache-layer
/// correctness. Callers who need completion guarantees should use
/// `fetch_query_with_signal` directly with their own completion handling.
pub fn use_query<T, E, C, F, Fut>(
    options: impl Into<crate::hook::QueryOptions>,
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
    // Audit H7: destructure opts so retry_policy can be moved (not cloned)
    // into the entity store; it has no later use. key is still cloned once for
    // use_query_manual (audit #61 moves the original into begin_request below).
    let crate::hook::QueryOptions {
        key,
        cache_policy,
        request_policy,
        retry_policy,
        force_fetch,
        ..
    } = options.into();
    let (entity, subscription) = use_query_manual(key.clone(), cache_policy, request_policy, cx);

    // Audit fix #16: Propagate the user's retry policy to the resource entity.
    // Without this, the resource defaults to RetryPolicy::no_retries() and
    // the user's QueryOptions::retry_policy() builder is a dead API.
    entity.update(cx, |r, _| r.set_retry_policy(retry_policy));

    // Start fetch if resource is idle
    let should_fetch = entity.read_with(cx, |r, _| r.status() == QueryStatus::Idle);
    if should_fetch {
        let fetch_mode = if force_fetch {
            QueryFetchMode::Force
        } else {
            QueryFetchMode::Normal
        };
        // Audit fix #3: begin_request_on_entity returns Option<RequestId>.
        // If CacheHit or IgnoredWhileLoading, skip spawning the fetch task.
        // Audit fix #2: Thread the key through to avoid re-reading from entity.
        // Audit fix #61: opts.key was cloned once above for use_query_manual
        // and is no longer needed after this call, so move it instead of
        // cloning again (removes a redundant second clone of the key).
        if let (Some(request_id), signal) =
            begin_request_on_entity(&entity, cx, fetch_mode, Some(key))
        {
            // Audit H3: `signal` comes straight from begin_request_on_entity
            // (read in the same entity.update as the begin) instead of via a
            // separate entity.read_with pass. unwrap_or_else covers the
            // pathological case where begin created no signal.
            let signal = signal.unwrap_or_else(QuerySignal::new);
            let weak = entity.downgrade();
            let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
            // Audit fix #6: store the spawned task on the resource so a
            // replacement fetch (or entity drop on unmount) aborts the prior
            // in-flight task instead of leaving it detached and running.
            let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
                fetch_signal_with_retry(fetcher, signal, request_id, &retry_policy, &weak, cx)
                    .await;
            });
            // Audit #6 NOT applied to queries: query fetches already prevent
            // stale writes via the signal + `is_current_request` cooperative
            // check in run_query_retry_loop, and tests enforce that a
            // superseded fetcher still observes its cancelled signal. Hard-
            // aborting on replacement would break that contract, so the task
            // is detached (it self-terminates when the entity is dropped).
            task.detach();
        }
    }

    (entity, subscription)
}

/// Like [`use_query`], but the fetcher returns [`Fetched<T>`] so a server-derived
/// [`CachePolicy`] can override the caller's per-query policy on success
/// ("server wins").
///
/// Mirrors [`use_query`] exactly — same options-first signature, same
/// [`QuerySignal`]-accepting fetcher, same
/// `(Entity<QueryResource<T, E>>, Subscription)` return — except the fetcher
/// returns `Result<Fetched<T>, E>`. [`Fetched::new`] keeps the caller's policy;
/// [`Fetched::with_policy`] overrides it with the server's (e.g. parsed from
/// `Cache-Control`) once the fetch resolves. The existing `Result<T, E>`
/// [`use_query`] is unchanged.
///
/// # Server wins
///
/// The resource's `CachePolicy` is established at `begin_request` time from
/// [`QueryOptions`] (the caller's policy). When a fetcher returns
/// [`Fetched::with_policy`], that server policy replaces the resource's stored
/// policy immediately after `complete_success`, so subsequent freshness / SWR
/// checks use the server's TTL. `None` (via [`Fetched::new`]) leaves the caller's
/// policy in place.
pub fn use_query_with_policy<T, E, C, F, Fut>(
    options: impl Into<crate::hook::QueryOptions>,
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
    let crate::hook::QueryOptions {
        key,
        cache_policy,
        request_policy,
        retry_policy,
        force_fetch,
        ..
    } = options.into();
    let (entity, subscription) = use_query_manual(key.clone(), cache_policy, request_policy, cx);

    // Audit fix #16 (mirrors `use_query`): propagate the user's retry policy.
    entity.update(cx, |r, _| r.set_retry_policy(retry_policy));

    // Start fetch if resource is idle
    let should_fetch = entity.read_with(cx, |r, _| r.status() == QueryStatus::Idle);
    if should_fetch {
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
            let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
            // Same deliberate detach as `use_query` (audit #6 NOT applied to
            // queries): cooperative signal cancellation + `accept_current_request`
            // prevent stale writes, and the task self-terminates on entity drop.
            let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
                fetch_signal_with_retry(fetcher, signal, request_id, &retry_policy, &weak, cx)
                    .await;
            });
            task.detach();
        }
    }

    (entity, subscription)
}

/// Like [`use_query`] but the fetcher receives no signal argument.
///
/// This exists for backward compatibility. Prefer [`use_query`] (the
/// signal-accepting version) which aligns with the v2 "Signal-always" design.
pub fn use_query_unsignalled<T, E, C, F, Fut>(
    key: QueryKey,
    cache_policy: crate::core::CachePolicy,
    request_policy: crate::core::RequestPolicy,
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
    let (entity, subscription) = use_query_manual(key.clone(), cache_policy, request_policy, cx);

    // Start fetch if resource is idle
    let should_fetch = entity.read_with(cx, |r, _| r.status() == QueryStatus::Idle);
    if should_fetch {
        // Audit fix #3: Only spawn fetch if begin_request returns a real RequestId.
        // Audit fix #2: Thread the key through to avoid re-reading from entity.
        if let (Some(request_id), _signal) =
            begin_request_on_entity(&entity, cx, QueryFetchMode::Normal, Some(key))
        {
            let weak = entity.downgrade();
            let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
            // Audit fix #6: store the task so replacement/unmount aborts it.
            let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
                fetch_with_retry(fetcher, request_id, &retry_policy, &weak, cx).await;
            });
            // Audit #6 NOT applied to queries: query fetches already prevent
            // stale writes via the signal + `is_current_request` cooperative
            // check in run_query_retry_loop, and tests enforce that a
            // superseded fetcher still observes its cancelled signal. Hard-
            // aborting on replacement would break that contract, so the task
            // is detached (it self-terminates when the entity is dropped).
            task.detach();
        }
    }

    (entity, subscription)
}

/// Convenience wrapper around [`use_query_manual`] that builds the entity and
/// observation from a [`QueryOptions`] value instead of raw policy parameters.
///
/// Audit fix #79: `use_query_manual` historically required a raw
/// `(key, cache_policy, request_policy)` triple. This overload accepts anything
/// convertible into [`QueryOptions`] (a string, a [`QueryKey`], a full
/// `QueryOptions` builder, or the legacy `(QueryKey, CachePolicy, RequestPolicy)`
/// tuple) so callers do not have to spell out the policies by hand. Only the
/// `key`, `cache_policy`, and `request_policy` fields are consumed; the
/// remaining options (retry policy, `force_fetch`, etc.) are ignored at this
/// layer — use [`use_query`] to honor them. The existing
/// [`use_query_manual`] signature is unchanged.
pub fn use_query_manual_opts<T, E, C>(
    options: impl Into<crate::hook::QueryOptions>,
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

/// Convenience wrapper around [`use_query_unsignalled`] that accepts an
/// `impl Into<QueryOptions>` instead of the raw `(key, cache_policy,
/// request_policy)` triple.
///
/// Audit fix #79: mirrors [`use_query_manual_opts`]. Only the `key`,
/// `cache_policy`, and `request_policy` fields of [`QueryOptions`] are read;
/// the remaining options are not consumed at this layer. The existing
/// [`use_query_unsignalled`] signature is unchanged.
pub fn use_query_unsignalled_opts<T, E, C, F, Fut>(
    options: impl Into<crate::hook::QueryOptions>,
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

/// Lower-level hook that sets up the entity and observation without starting a fetch.
///
/// Use this when you need full control over when and how fetching happens.
///
/// Uses v2's [`QueryObserver`] which returns `Option<Subscription>` instead of
/// panicking when the entity has been dropped.
///
/// # Panics (debug builds only)
///
/// In debug builds, panics if no [`QueryClient`] has been set via
/// `cx.set_global::<QueryClient>()`. In release builds, falls back to a
/// standalone entity (no shared caching, no GC) so that tests and demos
/// continue to work.
pub fn use_query_manual<T, E, C>(
    key: QueryKey,
    cache_policy: crate::core::CachePolicy,
    request_policy: crate::core::RequestPolicy,
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
        // Panic in debug builds when QueryClient is not initialized.
        // The silent fallback is appropriate for tests but dangerous for production.
        #[cfg(debug_assertions)]
        {
            eprintln!(
                "use_query_manual: no QueryClient set via cx.set_global(). \
                 Falling back to standalone entity (no shared caching, no GC). \
                 Call cx.set_global(QueryClient::new()) in your app setup."
            );
            panic!(
                "use_query_manual: QueryClient is not initialized. \
                 Call cx.set_global(QueryClient::new()) before using query hooks."
            );
        }
        #[cfg(not(debug_assertions))]
        {
            // Audit fix #5: Warning eprintln removed from release builds.
            // In release builds, silently fall back without leaking to stderr.
            cx.new(|_| QueryResource::new(key, cache_policy, request_policy))
        }
    };

    // Audit fix #12: Use match instead of expect() to avoid production panics.
    // In debug builds, the entity was just created so observe() should succeed.
    // In release builds, if GPUI internals change unexpectedly, fall back
    // gracefully rather than panicking.
    let observer = QueryObserver::new(&entity);
    let Some(subscription) = observer.observe(cx) else {
        #[cfg(debug_assertions)]
        panic!(
            "QueryObserver::observe failed: entity was just created and cannot be dropped. \
             This indicates a GPUI internal regression."
        );
        #[cfg(not(debug_assertions))]
        {
            // Audit fix #5: Warning eprintln removed from release builds.
            // Return a no-op subscription so the caller can continue.
            // This prevents a production panic from a GPUI internal issue.
            return (entity, Subscription::new(|| {}));
        }
    };

    (entity, subscription)
}

/// Initiate a fetch on an existing query entity.
///
/// Call this when you want to refetch (e.g., on button click or timer).
/// Respects the resource's retry policy on failure.
///
/// Calls `begin_request` to obtain a fresh `RequestId` and transition the
/// resource to Loading before spawning the fetch task.
///
/// Audit fix #3: If `begin_request` returns `None` (cache hit or ignored),
/// no async fetch task is spawned, avoiding wasted resources.
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
    // Audit fix #3: Only spawn fetch if begin_request returns a real RequestId.
    let (Some(request_id), _signal) =
        begin_request_on_entity(entity, cx, QueryFetchMode::Normal, None)
    else {
        return;
    };
    let weak = entity.downgrade();
    let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
    // Audit fix #6 (deliberate detach): plain-query fetches are NOT stored on
    // the resource. Cooperative signal cancellation + `accept_current_request`
    // already prevent stale writes (see the module-level docs), so the task is
    // detached and self-terminates once the entity is dropped.
    let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
        fetch_with_retry(fetcher, request_id, &retry_policy, &weak, cx).await;
    });
    task.detach();
}

/// Like [`fetch_query`], but the fetcher returns [`Fetched<T>`] so a server-derived
/// [`CachePolicy`] can override the resource's policy on success ("server wins").
///
/// See [`use_query_with_policy`] for the server-wins semantics. Respects the
/// resource's retry policy on failure.
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
    // Audit fix #3: Only spawn fetch if begin_request returns a real RequestId.
    let (Some(request_id), _signal) =
        begin_request_on_entity(entity, cx, QueryFetchMode::Normal, None)
    else {
        return;
    };
    let weak = entity.downgrade();
    let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
    // Same deliberate detach as `fetch_query` (audit #6 NOT applied to queries).
    let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
        fetch_with_retry(fetcher, request_id, &retry_policy, &weak, cx).await;
    });
    task.detach();
}

/// Like [`fetch_query`], but the fetcher receives a [`QuerySignal`] that it can
/// check periodically for cooperative cancellation.
///
/// The fetcher signature is `FnOnce(QuerySignal) -> Fut`. Since `FnOnce` closures
/// are consumed on the first call, retries are not possible.
///
/// Calls `begin_request` to obtain a fresh `RequestId` and reads the signal
/// *after* `begin_request` creates it (v2 fix for stale signal).
///
/// Audit fix #3: If `begin_request` returns `None`, no async fetch task is spawned.
///
/// # Signal Cancellation (Audit Finding #8)
///
/// The `accept_current_request` guard is the authoritative protection against
/// stale writes. A previous `signal.is_cancelled()` check after the fetcher
/// returned was removed -- it was a best-effort optimization with a TOCTOU
/// window that provided no guarantees. The two-phase protocol (accept + complete)
/// correctly handles all cases where a newer request supersedes the current one.
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
    // Audit fix #3: Only spawn fetch if begin_request returns a real RequestId.
    let (Some(request_id), signal) =
        begin_request_on_entity(entity, cx, QueryFetchMode::Normal, None)
    else {
        return;
    };
    // Audit H3: `signal` is the one begin_request just created, read in the
    // same entity.update as the begin (no separate read pass).
    let signal = signal.unwrap_or_else(QuerySignal::new);
    let weak = entity.downgrade();

    // FnOnce fetchers can only be called once, so retries are not possible.
    // Audit fix #6 (deliberate detach): plain-query fetches are NOT stored on
    // the resource. The `accept_current_request` guard below is the
    // authoritative protection against stale writes (see the module-level
    // docs), so the task is detached and self-terminates once the entity is
    // dropped.
    let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
        let result = fetcher(signal).await;

        let now_ms = current_time_ms();
        let Some(entity) = weak.upgrade() else { return };

        // Audit fix #7/#13: Only call cx.notify() when the result is actually
        // accepted. When accept_current_request returns None, no state change
        // occurred and no re-render is needed.
        //
        // Audit fix #8: Removed the signal.is_cancelled() check. The
        // accept_current_request guard is the authoritative protection.
        let _ = entity.update(cx, |resource, cx| {
            if let Some(guard) = resource.accept_current_request(request_id) {
                match result {
                    Ok(data) => {
                        resource.complete_success(guard, data, now_ms);
                    }
                    Err(error) => {
                        resource.complete_failure(guard, error, now_ms);
                    }
                }
                cx.notify();
            } else {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DEBUG: fetch_query_with_signal: request {} no longer active, result discarded",
                    request_id.label()
                );
            }
        });
    });
    task.detach();
}

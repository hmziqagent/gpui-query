---
name: gpui-query
description: Use when building a GPUI app that depends on gpui-query (v0.2.0) — writing use_query / use_mutation / use_infinite_query / use_query_select hooks; configuring in-memory CachePolicy (NoCache/Ttl/StaleWhileRevalidate) or RetryPolicy; constructing QueryKey / QueryKeyFilter; calling QueryClient for fetch_query / prefetch / set_query_data / invalidate_queries / cancel_queries / reset_queries / remove_queries; wiring cx.set_global(QueryClient::new()); or debugging observer re-render / stale-write / GC behavior. Do NOT use for general GPUI app work that does not involve gpui-query, for the HTTP-cache/disk-persistence satellites (use the gpui-query-extensions skill), or for editing the gpui-query crate itself (see AGENTS.md for crate-internal work).
---

# gpui-query (essential)

Async state management for [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), inspired by TanStack Query v5. You write a fetcher; the library caches, retries, deduplicates, invalidates, garbage-collects, and cooperatively cancels. Crate: `gpui-query` v0.2.0. This skill covers the in-memory core/client/hook tiers (persistence and HTTP-cache satellites are separate).

## Install

Three strictly-additive tiers, glob re-exported at the crate root (`pub use core::*; pub use client::*; pub use hook::*;`) — import everything from `gpui_query::`.

| Tier | Cargo line | What you get |
|---|---|---|
| core only (no GPUI) | `gpui-query = { version = "0.2.0", default-features = false, features = ["core"] }` | `QueryResource` state machine, `CachePolicy`, `RetryPolicy`, `QueryKey`, `QuerySignal`. Zero GPUI dep — usable in non-GPUI libs. |
| client (DEFAULT) | `gpui-query = "0.2.0"` | + `QueryClient` GPUI `Global`: type-partitioned buckets, GC, bulk invalidate/cancel/reset/remove, observers, `PreparedFetch`. |
| hooks | `gpui-query = { version = "0.2.0", features = ["hook"] }` | + `use_query` / `use_mutation` / `use_infinite_query` / `use_query_select`. |

The `client` tier pulls `gpui = "0.2.2"`. macOS builds of any tier with GPUI need the Metal Toolchain installed once: `xcodebuild -downloadComponent MetalToolchain` (core-only builds need nothing).

## Mental model

**Two-phase fetch lifecycle.** Every fetch is gated by a monotonic `RequestId` minted per-resource by a `RequestSequencer` co-located in the bucket. The lifecycle:

1. `begin_request` → `QueryBeginResult` (`Started { request_id, .. }` | `CacheHit` | `StaleCacheHit { request_id, .. }` | `IgnoredWhileLoading`). Transitions status to `LoadingEmpty`/`LoadingWithData` and installs a fresh `QuerySignal`.
2. Caller fetches async (cooperative: poll `signal.is_cancelled()` to abort early).
3. `accept_current_request(request_id) -> Option<RequestGuard>` — `Some` ONLY if `request_id` is still the active one. A superseded fetch gets `None`.
4. `complete_success(guard, data, now_ms)` / `complete_failure(guard, error, now_ms)` consume the single-use `RequestGuard` by value.

**`RequestGuard` is the authoritative stale-write protection.** A cancelled or superseded async task can `accept` → `None` and its result is silently discarded. Do NOT rely on a `signal.is_cancelled()` check after the fetch returns (TOCTOU window) — the guard is what guarantees correctness. The hooks wrap all of this for you; you only touch `accept`/`complete` directly via `fetch_query_with_signal` or `PreparedFetch`.

**Status states** (`QueryStatus`, `#[derive(Default)]` so `Idle` is the start):

```
Idle → LoadingEmpty → Success | Failure
Success → LoadingWithData → Success | Failure   (refetch)
```

- `is_loading()` = `LoadingEmpty | LoadingWithData`
- `is_pending()` = `Idle | LoadingEmpty` (TanStack `isPending` parity)
- Plus `Cancelled` (explicit cancel).

Plain-query fetch tasks are `.detach()`ed — the signal + guard already prevent stale writes, and the task self-terminates when its `WeakEntity` target is dropped. Mutations and infinite queries store the task via `set_current_task`, so a replacement call or component unmount HARD-aborts the prior task.

## Quick start

```rust
use gpui_query::{use_query, QueryClient};

// Once, at app startup. In debug builds the `use_query` family panics without
// this (`use_infinite_query` / `use_mutation` fall back silently); release
// builds always fall back to standalone entities (no shared cache, no GC, no
// bulk ops). Always set it.
cx.set_global(QueryClient::new());

struct UserList {
    users: gpui::Entity<gpui_query::QueryResource<Vec<User>, MyError>>,
    _subscription: gpui::Subscription,   // store both — see Observers
}

impl UserList {
    fn new(cx: &mut gpui::Context<Self>) -> Self {
        let (users, sub) = use_query(
            "users",                              // &str: Into<QueryOptions>
            |signal| async move {
                if signal.is_cancelled() { return Err(MyError::Cancelled); }
                Ok(fetch_users().await?)
            },
            cx,
        );
        Self { users, _subscription: sub }
    }
}
```

Every hook returns `(Entity<Resource>, Subscription)` — both must be stored. Dropping the `Subscription` kills the observation and the component stops re-rendering on state changes.

## use_query / use_query_with_policy

```rust
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
    Fut: Future<Output = Result<T, E>> + Send + 'static;
```

`use_query_with_policy` is identical except `Fut: Future<Output = Result<Fetched<T>, E>>` — the fetcher returns a `Fetched<T>` so a server-derived `CachePolicy` overrides the resource's on success (**"server wins"**):

```rust
use gpui_query::{use_query_with_policy, QueryOptions, Fetched, CachePolicy};

let (entity, _sub) = use_query_with_policy(
    QueryOptions::new("user/42"),
    |signal| async move {
        let (user, cc) = fetch_user_with_cache_control().await?;
        // Fetched::with_policy overrides the resource's policy with the
        // server's (e.g. parsed from Cache-Control). Fetched::new(user)
        // keeps the caller's policy unchanged.
        Ok(Fetched::with_policy(user, cc))
    },
    cx,
);
```

`Fetched::new(data)` keeps the caller's policy; `Fetched::with_policy(data, policy)` replaces it after `complete_success`, so subsequent freshness/SWR checks use the server's TTL.

`QueryOptions` builder (also `From<&str>` / `From<String>` / `From<QueryKey>` / `From<(QueryKey, CachePolicy, RequestPolicy)>`):

| Builder | Effect |
|---|---|
| `.cache_policy(p)` | Per-query cache policy (default `Ttl { ttl_ms: 60_000 }`). |
| `.request_policy(p)` | `LatestWins` (default) or `IgnoreWhileLoading`. |
| `.retry_policy(p)` | Per-query retry (default 3 + exponential). Installed onto the entity — without a hook the resource starts at `RetryPolicy::no_retries()`. |
| `.force()` | `force_fetch = true` → bypass cache freshness, always fetch. |
| `.gc_time(ms)` / `.keep_previous()` | **Reserved / forward-compat.** Stored, NOT consumed today (GC runs off `QueryClient::with_gc_time`). |

**Imperative refetch** on an existing entity (button click, timer, after invalidation): `fetch_query(&entity, || async { ... }, cx)`. Variants: `fetch_query_with_policy` (server-wins) and `fetch_query_with_signal` (`FnOnce(QuerySignal) -> Fut`; **no retries** because `FnOnce` is consumed on first call).

## use_mutation

```rust
pub fn use_mutation<V, T, E, C>(
    options: impl Into<MutationOptions>,          // use_mutation((), cx) works via From<()>
    cx: &mut Context<C>,
) -> (Entity<MutationResource<V, T, E>>, Subscription)
where V: Clone + Send + Sync + 'static, T: Clone + Send + Sync + 'static,
      E: Clone + Send + Sync + 'static, C: 'static;
```

`MutationResource<V, T, E>`: `V` = variables (input), `T` = success output, `E` = error (defaults to `QueryError`). Status: `Idle` → `Loading` → `Success` | `Failure`.

Trigger with `mutate` (or `mutate_with_callbacks`):

```rust
pub fn mutate<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: V,
    mutator: F,                                  // Fn(V) -> Fut
    cx: &mut Context<C>,
) where ...;

pub fn mutate_with_callbacks<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: V,
    mutator: F,
    callbacks: MutationCallbacks<T, E>,
    cx: &mut Context<C>,
);
```

`MutationCallbacks::new().on_success(|t: &T| ...).on_error(|e: &E| ...).on_settled(|opt_t: Option<&T>, opt_e: Option<&E>| ...)`. The closures are `Fn(&T)` / `Fn(&E)` / `Fn(Option<&T>, Option<&E>)` (borrowed, not owned) — they fire on the terminal outcome (after retries exhaust / first success), run outside any entity borrow, and are safe to call `entity.update()` inside.

```rust
use gpui_query::hook::{use_mutation, mutate_with_callbacks, MutationCallbacks};

// In handle_submit:
mutate_with_callbacks(
    &self.create_user,
    NewUser { name },
    |vars| async move { api_create_user(vars).await },
    MutationCallbacks::new()
        .on_success(|_user: &User| { /* navigate, etc. */ })
        .on_settled(|_, _| { /* always: hide spinner */ }),
    cx,
);
```

Behavior:
- **Concurrent guard:** `mutate` does an atomic check+`begin` inside one `entity.update`. If already `Loading`, the call is a no-op (no second in-flight mutation on the same entity).
- **Drop safety net:** if the entity is dropped mid-mutation, `on_error` / `on_settled(None, None)` still fire so callers always get a terminal callback.
- **Hard-abort:** the task is stored via `set_current_task`; a replacement `mutate` or component unmount aborts the prior task.
- `mutate_by_ref` / `mutate_arc` — mutator takes `&V` (borrowed from a stored `Arc<V>`), so the retry loop does **no `V::clone` per attempt**. `V: Clone` is still required (the initial `begin` stores one owned copy).
- `use_mutation_state::<V,T,E,_>(cx)` returns all registered mutation entities of that type triple (for devtools / batch views).

**Optimistic update pattern** — write to the cache before the mutation resolves, then invalidate/refetch on settle:

```rust
// Before mutate: prime the cache so the UI updates instantly.
cx.update_global::<gpui_query::QueryClient, _>(|c, cx| {
    c.set_query_data::<Vec<User>, MyError>(
        "users",
        vec![User { name: name.clone(), ..Default::default() }],  // optimistic
        cx,
    );
});
// On settle: invalidate_queries so the real fetcher overwrites it.
```

`set_query_data` saves the previous data; the resource exposes `rollback_to_previous()` if you need to undo on failure.

## use_infinite_query

```rust
pub fn use_infinite_query<T, E, C, FNext, Fut>(
    options: InfiniteQueryOptions,               // concrete (not impl Into)
    fetch_next: FNext,
    cx: &mut Context<C>,
) -> (Entity<InfiniteQueryResource<T, E>>, Subscription)
where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    FNext: Fn(Option<&T>) -> Fut + 'static,      // receives the last page (or None)
    Fut: Future<Output = Result<(T, bool), E>> + Send + 'static;  // (page, has_more)
```

The fetcher returns `(page_data, has_more)`. `Option<&T>` is the previously-loaded last page so the fetcher can derive a cursor. Drive pagination imperatively:

```rust
use gpui_query::{use_infinite_query, fetch_next_page_infinite, InfiniteQueryOptions};

let (feed, _sub) = use_infinite_query(
    InfiniteQueryOptions::new("feed").max_pages(20),
    |last: Option<&Vec<Post>>| async move {
        let cursor = last.and_then(|p| p.last().map(|x| x.id)).unwrap_or(0);
        let (posts, more) = fetch_page(cursor).await?;
        Ok((posts, more))
    },
    cx,
);

// On scroll-to-bottom:
fetch_next_page_infinite(&self.feed, |last| async move { /* same shape */ }, cx);
// Backward: fetch_previous_page_infinite(&entity, |first| async move { ... }, cx);
```

`InfiniteQueryOptions`: `.max_pages(n)` (default `Some(50)`, bounded to prevent unbounded growth), `.unbounded_pages()` (`None` — never evict; use with caution), plus `.cache_policy()` / `.retry_policy()` / `.gc_time()` builders. Construct via `InfiniteQueryOptions::new(key)` or `InfiniteQueryOptions::from("feed")`.

`InfiniteQueryResource<T, E>` accessors:

| Method | Returns |
|---|---|
| `pages()` | `&VecDeque<Arc<T>>` — all loaded pages, first→last. |
| `page_count()` / `has_data()` | page count / any loaded. |
| `first_page()` / `last_page()` | `Option<&T>` borrowed views. |
| `first_page_arc()` / `last_page_arc()` | `Option<Arc<T>>` — cheap refcount bump to hand to a fetcher. |
| `has_next_page()` / `has_previous_page()` | more pages available in either direction. |
| `is_fetching_next_page()` / `is_fetching_previous_page()` | a page fetch is in flight. |
| `is_page_data_valid()` | `true` on `Success`/`LoadingWithData`; on `Failure` returns `true` if pages exist (the failure is scoped to the last page fetch — prior pages stay valid). |

`FetchDirection` (set at construction, not by the hook options): `ForwardOnly` (default — `has_next_page` starts `true` as an assumption, fetcher's `has_more` drives it false) vs `Bidirectional` (both start `false`; use `InfiniteQueryResource::new_bidirectional`). The default hook uses `ForwardOnly`.

## use_query_select

Project cached data through a `SelectTransform<T, U>` — multiple derived views over one cache entry, no duplication.

```rust
pub type QuerySelectResult<T, U, E> = (
    Entity<MappedQueryResource<T, U, E>>,
    Entity<QueryResource<T, E>>,
    (Subscription, Subscription),            // query sub + mapped observer sub
);

pub fn use_query_select<T, U, E, C, F, Fut>(
    options: impl Into<QueryOptions>,
    transform: SelectTransform<T, U>,        // SelectTransform::new(|t: &T| -> U)
    fetcher: F,
    cx: &mut Context<C>,
) -> QuerySelectResult<T, U, E>
where T: Clone + PartialEq + Send + Sync + 'static, U: 'static, /* ... */;
```

```rust
use gpui_query::{use_query_select, QueryOptions, core::SelectTransform};

let count = SelectTransform::new(|users: &Vec<User>| users.len());
let (mapped, query_entity, subs) = use_query_select(
    QueryOptions::new("users"),
    count,
    |signal| async move { Ok(fetch_users().await?) },
    cx,
);
// mapped.read(cx).data()  -> Option<usize>
```

Store **both** subscriptions (`(query_sub, mapped_sub)`) or the projection stops updating. The transform closure runs on every `MappedQueryResource::data()` call (no output cache) — for expensive transforms, bind `let data = mapped.read(cx).data();` once per render and reuse. The mapped entity re-syncs from the source only when the source `T` actually changed (`PartialEq`), so unchanged notifications pay just an `Arc::clone`.

## CachePolicy

`#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]`. Default (via `QueryOptions` and `QueryClient`): `Ttl { ttl_ms: 60_000 }`.

| Variant | Behavior | Freshness |
|---|---|---|
| `NoCache` | Always fetch; never short-circuits. | `is_fresh` always false; `is_expired` always true. |
| `Ttl { ttl_ms }` | Serve fresh within TTL. | `is_fresh(age)` when `age <= ttl_ms` — **INCLUSIVE boundary** (opposite of HTTP `max-age`; mind the off-by-one). |
| `StaleWhileRevalidate { ttl_ms, stale_ms }` | `[0, ttl]` fresh; `[ttl, ttl+stale]` serve stale + background revalidate; past `ttl+stale` expired. | `is_stale_but_serveable(age)` in the stale window. |

Helper methods: `can_short_circuit()` (Ttl/SWR), `can_serve_stale()` (SWR only), `is_fresh(age)`, `is_stale_but_serveable(age)`, `is_expired(age)`, `ttl_ms()`, `total_valid_ms()` (Ttl → `ttl_ms`; SWR → `ttl_ms + stale_ms` saturating). `ttl_ms == 0` behaves like `NoCache` (only `debug_assert`s in debug builds).

## RetryPolicy

All four fields public; builder pattern.

```rust
pub struct RetryPolicy {
    pub max_retries: u32,              // 0 = no retries
    pub retry_delay_ms: u64,           // base delay
    pub exponential_backoff: bool,
    pub max_retry_delay_ms: u64,       // configured cap
}
```

| Constructor / builder | Result |
|---|---|
| `RetryPolicy::no_retries()` | `max_retries: 0`. (This is what a bare `QueryResource::new` starts with — the hook installs the real policy.) |
| `RetryPolicy::new(n)` | `max_retries: n`, base 1000ms, no exponential, cap 30_000ms. |
| `RetryPolicy::default()` | `new(3).with_exponential_backoff()` — **3 retries, exponential, 1s base, 30s cap.** |
| `.with_delay(ms)` / `.with_exponential_backoff()` / `.with_max_delay(ms)` | mutators. |

`delay_for_attempt(attempt)` (0-based):

- exponential off → constant `retry_delay_ms`.
- exponential on → `retry_delay_ms * 2^attempt`, where the shift is clamped to 62 (prevents overflow) and the multiply is saturating; then `.min(max_retry_delay_ms).min(3_600_000)` — a **hard 1-hour absolute ceiling** regardless of config.

`should_retry(current_retries)` = `current_retries < max_retries`. Mutations and queries reset `retry_count` on a new `begin`, so each invocation gets fresh retries.

## QueryKey + QueryKeyFilter

`QueryKey` — hierarchical key backed by `Arc<[Arc<str>]>` (clone = one refcount bump regardless of length). Serde-flexible (deserializes from a JSON string array OR a bare string).

```rust
// Construction
QueryKey::from("users")                         // From<&str> -> single segment
QueryKey::from(["users", "42", "posts"])        // From<[&str; N]> -> multi-segment
QueryKey::from_single("users")
QueryKey::new(["users", &id.to_string()])       // PANICS on an empty iterator — guard emptiness
```

Accessors: `parts() -> &[Arc<str>]`, `as_single() -> Option<&str>` (only if exactly one segment), `first_segment() -> &str` (first only — NOT the joined key), `to_path() -> String` (`"::"`-joined, e.g. `"users::42::posts"`), `join(extra) -> QueryKey` (O(n) copy — prefer one `new([...])` over chained `.join()`). `starts_with(prefix)` is **segment-wise** (used by `Prefix` filters); an empty prefix matches every valid key.

`QueryKeyFilter<'a>` (used by every bulk op on `QueryClient`):

| Variant | Matches |
|---|---|
| `Exact(&QueryKey)` | only that exact key. |
| `Prefix(&QueryKey)` | all keys that `starts_with` it (segment-wise — `["users"]` matches `["users", "42", "posts"]`). |
| `All` | every key. |

## QueryClient API

A GPUI `Global`. Set once: `cx.set_global(QueryClient::new())`. Access from anywhere: `cx.global::<QueryClient>()` / `cx.update_global::<QueryClient, _>(|c, cx| ...)`.

**Construction:**
- `QueryClient::new()` — defaults (`Ttl 60s`, `LatestWins`, `gc_time_ms: 300_000`).
- `QueryClient::with_policies(cache, request)` — custom default policies for resources created via `resource()` (per-query `QueryOptions` still overrides).
- `.with_gc_time(ms)` — `0` disables **opportunistic** (automatic) GC only; an explicit `gc(cx)` / `gc_with_time(now, cx)` still runs (clamping any value `< 1000` to `1000` during the pass).

**Resource access** (type-partitioned by `(T, E)`):
```rust
client.resource::<Vec<User>, MyError>("users", cx)                  // get-or-create with default policies
client.resource_with_policies::<T, E>(key, cache, request, cx)      // explicit policies
client.query::<T, E>(&key)            // Option<Entity<QueryResource<T, E>>>
client.all_queries::<T, E>()          // Vec<Entity<...>>  (allocates)
```

**Data accessors** (ergonomic cache reads/writes; type params required):
```rust
client.get_query_data::<Vec<User>, MyError>(&"users", cx)           // Option<Vec<User>> (clones T)
client.with_query_data::<Vec<User>, MyError, _>(&"users", cx, |u: &Vec<User>| u.len())  // Option<R>, zero-clone
client.set_query_data::<Vec<User>, MyError>("users", data, cx)      // creates resource if absent; saves previous for rollback
```

**Bulk operations** (take `&QueryKeyFilter`, operate across all query + infinite buckets):
```rust
client.invalidate_queries(&filter, cx)   // clears last_updated_at; DATA RETAINED but now stale -> next read refetches
client.reset_queries(&filter, cx)        // back to Idle, clears data/error
client.remove_queries(&filter)           // drops entries entirely (no cx needed)
client.cancel_queries(&filter, cx)       // cooperative-cancel in-flight (signal.cancel() + mark_ignored_result)
```
`invalidate_queries` is the workhorse after a mutation: it marks matching keys stale so the next `use_query` mount or read refetches, without dropping the cached value (avoids a loading flash).

**GC and diagnostics:**
- `client.gc(cx)` / `gc_with_time(now_ms, cx)` — evicts dead refs; `Idle`/`Failure`/`Cancelled` older than `gc_time_ms`; `Success` older than `gc_time_ms * SUCCESS_GC_MULTIPLIER`; loading resources always retained. GC also fires opportunistically every `GC_INTERVAL` operations (debounced by `MIN_GC_TIME_MS`), so you rarely call it manually.
- `client.diagnostics(cx) -> ClientDiagnostic` — per-resource key/status/cache-age for devtools.

**Imperative fetch / prefetch** (no observer attached — for warming the cache outside a component):
```rust
// fetchQuery: forced, always returns a PreparedFetch (None only if no sequencer).
if let Some(p) = client.prepare_fetch_query::<UserData, MyError>("user/42", cx) {
    let signal = p.signal.clone();
    let entity = p.entity.clone();
    cx.spawn(async move |_, cx| {
        match fetch(signal).await {
            Ok(data) => p.complete_success(data, cx),   // no-op if superseded
            Err(e)    => p.complete_failure(e, cx),
        }
    }).detach();
}

// prefetchQuery: respects cache_policy — returns None on a fresh cache hit.
if let Some(p) = client.prepare_prefetch_query::<T, E>(key, cache, request, cx) { /* ... */ }
```

`PreparedFetch<T, E>` is `#[must_use]` — it holds the entity, `request_id`, `signal`, and the captured `now_ms` (used as the logical completion time). `complete_success(data, cx)` / `complete_failure(err, cx)` consume it by value and are no-ops if the request was superseded.

### Manual `QueryResource` / `MutationResource` control

Beyond hooks, the entity itself exposes (call inside `entity.update(cx, |r, cx| …)`):

- `cancel(error: E) -> bool` — Loading → Cancelled, stashes current data into `previous_data`. **No-op (returns `false`) unless currently Loading.** `MutationResource::cancel(error: E)` has the same shape — cancel takes an `E`, not unit.
- `reset()` — back to Idle; clears data / error / diagnostic counters, preserves policies and key.
- `set_data(data)` / `clear_data()` — optimistic primitives (write or clear without a fetch); `rollback_to_previous()` undoes a `set_query_data` / `set_data`.
- `is_data_stale(now_ms)` — staleness heuristic; `complete_current_success(request_id, data, now_ms)` / `complete_current_failure(request_id, err, now_ms)` do accept + complete in one call for manual flows.

## Observers & re-render semantics

A single generic `Observer<R: ObservableResource>` with aliases `QueryObserver<T, E>`, `InfiniteQueryObserver<T, E>`, `MutationObserver<V, T, E>`. The hooks attach one automatically; manual use:

```rust
let sub = QueryObserver::new(&entity)
    .with_config(ObserverConfig { notify_on_status_change_only: true })  // default
    .observe(cx)?;   // Option<Subscription> — None if entity already dropped
```

**Re-renders fire ONLY when `observable_status()` changes.** `increment_retry()` / `prepare_retry()` (status stays `Loading`) and `set_current_task` do NOT re-render — this is why a 3-retry mutation doesn't paint 3 times. Set `notify_on_status_change_only: false` to get a notify on every entity mutation.

**Subscription retention is mandatory.** A hook's returned `Subscription` must outlive the component's interest; dropping it detaches the observer and the component stops reacting. `use_query_select` returns **two** subscriptions (query + mapped observer) — store both, usually as a tuple field.

## Gotchas

- **Inclusive TTL boundary.** `is_fresh(age)` is `age <= ttl_ms` (not `<`) — opposite of HTTP `max-age`. A `Ttl { ttl_ms: 1000 }` is still fresh at exactly 1000ms. Watch for off-by-one when translating server `Cache-Control`.
- **Empty-key panic.** `QueryKey::new(iter::empty())` panics unconditionally in all build modes. Use `from_single` / `From<&str>` / guard the iterator at the call site.
- **`QueryClient` required in debug.** `use_query_manual` — and the `use_query` / `use_query_with_policy` / `use_query_unsignalled` / `use_query_manual_opts` hooks that delegate to it — **panics** in debug builds if no global `QueryClient` is set; release silently falls back to a standalone entity. `use_infinite_query` and `use_mutation` do NOT panic: in debug `use_infinite_query` prints an `eprintln!` warning and falls back to a standalone entity, and `use_mutation` silently skips client registration (so no shared cache, no GC, no bulk ops, no `use_mutation_state`). Always `cx.set_global(QueryClient::new())` at startup.
- **Subscription drop kills observation.** Every hook returns `(Entity, Subscription)`; `use_query_select` returns a 3-tuple with two subs. Bind them as struct fields (`_subscription`), not temporaries, or the component freezes after first render.
- **`WeakEntity` silent discard.** Async tasks capture `entity.downgrade()`. If the owning component unmounts mid-fetch, `weak.upgrade()` returns `None` and the result is silently discarded — no callback fires. Mutations are the exception: `on_settled(None, None)` / `on_error` still fire as a safety net. For completion guarantees on queries, use `fetch_query_with_signal` with your own handling.
- **`QueryResource::new` starts with `no_retries()`.** The `use_query` hook installs the real `RetryPolicy` from options; if you construct resources directly (or call `fetch_query` on a manually-made entity), set it yourself via `entity.update(cx, |r, _| r.set_retry_policy(p))`.
- **Mutation GC actually runs.** `QueryClient`'s explicit `Default` sets `gc_time_ms: 300_000` (the derived `Default` would have been `0` = disabled). `with_gc_time(0)` still disables; any value `< 1000` is clamped to `1000` during the pass. Mutations are GC-eligible by completion time, not insertion time.
- **`mutate` is a no-op when Loading.** The check+`begin` is atomic inside one `entity.update`; a racing second call cannot slip through. If you need to replace an in-flight mutation, cancel or reset first.
- **`max_pages` default is bounded.** `InfiniteQueryOptions` defaults to `Some(50)`. `ForwardOnly` (default direction) starts `has_next_page = true` as an assumption — the fetcher's `has_more` is what flips it false.
- **`MappedQueryResource::data()` re-runs the transform every call.** No output cache. Bind the result to a local once per render.
- **`cancel_queries` is two-step per match.** It reads `is_loading()` before mutating so non-loading entries don't get spurious observer notifications.
- **macOS Metal Toolchain.** Any tier pulling `gpui` (`client`/`hook`/`persist`) fails to build without it. `xcodebuild -downloadComponent MetalToolchain` once. Core-only builds are unaffected.

## Pointers

- Docs site: https://gpui-query.freeoxide.com/docs/ — guides under `/docs/guides/` (`caching`, `retry`, `query-keys`, `select-pattern`, `error-handling`); API reference under `/docs/api/`.
- Public API surface: everything is glob re-exported at the crate root. The `core::`, `client::`, and `hook::` modules are also public if you need a fully-qualified path (e.g. `gpui_query::core::SelectTransform`, `gpui_query::client::PreparedFetch`).
- Type parameters are load-bearing: `QueryResource<T, E>`, `MutationResource<V, T, E>` (E defaults to `QueryError`), `InfiniteQueryResource<T, E>`. `(T, E)` is the bucket partition key — the same key under two different type pairs is two separate cache entries.

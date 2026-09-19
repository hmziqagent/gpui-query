---
name: gpui-query-extensions
description: Use when adding HTTP cache-header handling (RFC 9111 Cache-Control -> CachePolicy, HttpCache<B>, ReqwestBackend, conditional GETs/304s) or durable disk persistence (FilePersister, async Persister trait, persist_with debounced driver, hydrate, typed serializer/deserializer registries) to a gpui-query app. Do not use for the essential in-memory cache alone, for general GPUI app work, or for editing the gpui-query crates themselves (see AGENTS.md for crate-internal work).
---

# gpui-query extensions: HTTP caching & disk persistence

The main crate ships an in-memory, type-partitioned cache with GC, invalidation, and TTL/SWR policies. Two **satellite crates** + one **main-crate feature** add cross-restart durability and server-driven HTTP cache semantics. Everything here is strictly additive over `client`.

- `gpui-query-http` (v0.1.0) — RFC 9111 `Cache-Control` → `CachePolicy` ("server wins"), plus a URL-keyed in-memory `HttpCache<B>` for cheap `304` revalidations. **GPUI-free** (depends on `core` only).
- `gpui-query-persist` (v0.1.0) — reference atomic-durable `FilePersister`.
- main crate `persist` feature — the async `Persister` trait, `persist_with` debounced driver, `hydrate`, and the typed serializer/deserializer registries.

Reach for these when:
- the app should **survive a restart** with its cached data primed (cold start shows last-known-good instead of a loading spinner);
- a query fetches over **HTTP** and you want the server's `max-age`/`stale-while-revalidate` to drive the resource's `CachePolicy`, and `ETag`/`Last-Modified` to make refetches cheap.

Do NOT reach for them for ephemeral in-memory state, or if you only need client-side TTLs the app controls itself (use `CachePolicy` directly on the resource).

## Install

```toml
[dependencies]
gpui      = "0.2.2"
gpui-query = { version = "0.2.0", features = ["persist"] }   # enables persist layer

# HTTP cache (optional reqwest backend):
gpui-query-http = { version = "0.1", features = ["reqwest"] } # drop "reqwest" to use your own HttpBackend

# Reference disk persister:
gpui-query-persist = "0.1"
```

The `persist` feature is `client + hook + dep:serde_json + dep:thiserror`. `gpui-query-persist` hard-depends on `persist + client + hook`. `gpui-query-http`'s `reqwest` feature pulls `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }`. With `default-features = false`, reqwest drops its own defaults (`default-tls` = native-tls, plus `charset`, `http2`, `macos-system-configuration`) and only rustls TLS is added back. Cookies and gzip/brotli/deflate compression are opt-in reqwest features that were **never** on by default — enable them on your own `reqwest::Client` if you need them.

macOS note: building `client`/`hook`/`persist` needs the Metal Toolchain once (`xcodebuild -downloadComponent MetalToolchain`). `gpui-query-http` (core-only) needs nothing extra.

---

## Persistence mental model

```
register (de)serializers  ──►  hydrate() at cold start  ──►  persist_with() reacts to cache writes
        (TypeId-keyed)           (load snapshot,                  (debounced save on every
                                 re-prime concrete T)              CacheMutation bump)
```

1. **Register** a serializer and deserializer per resource data type `T`. Only resources with a registered serializer are emitted into the snapshot; unregistered types fall back to metadata-only (skipped).
2. **`hydrate()`** at startup: load the snapshot, offer each on-disk entry to every registered deserializer, prime matches via `set_query_data::<T, E>`.
3. **`persist_with()`**: install a drop-guard driver. Every `CacheMutation` bump (a query/mutation resolving, `set_query_data`, `invalidate`, GC eviction) collects a fresh snapshot on the main thread, stashes it in a single pending slot, and spawns a debounced `save` on GPUI's `background_executor`. Bursts coalesce — only the latest snapshot survives the window.

The snapshot value is an opaque `serde_json::Value`; the typed round-trip is driven by the registries, so core never needs a `T: Serialize` bound.

---

## Persister trait + snapshot types

Main crate, `gpui_query::client::*` (re-exported at the crate root via `pub use client::*`).

```rust
pub const PERSIST_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("persistence io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("persistence serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("persistence deserialize error: {0}")]
    Deserialize(String),                 // reserved for backends that surface parse errors
    #[error("persistence version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },
    #[error("persistence bad path: {0}")]
    BadPath(String),
    #[error("persistence permission denied: {0}")]
    Permission(String),                  // retryable (Windows AV / lock contention)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistedEntry {
    pub value: serde_json::Value,        // opaque; typed round-trip via registries
    pub cached_at: u64,                  // wall-clock ms since UNIX epoch
    pub cache_policy: CachePolicy,
    pub meta: Option<serde_json::Value>, // reserved for HTTP CacheMeta, etc.
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PersistSnapshot {
    pub entries: std::collections::HashMap<String, PersistedEntry>, // keyed by QueryKey::to_path()
    pub version: u32,
}
impl PersistSnapshot { pub fn new() -> Self { /* empty, at PERSIST_VERSION */ } }
```

The trait is **async + non-object-safe** (methods return `impl Future + Send`, not `Pin<Box<dyn Future>>`). It is consumed generically by `persist_with<P: Persister>`, which monomorphizes the driver around the concrete `P`. This keeps `Send + 'static` visible at the call site (the save future runs on GPUI's `background_executor`) and avoids boxed-future overhead. Consequence: you cannot hold a `dyn Persister`.

```rust
pub trait Persister: Send + Sync + 'static {
    fn load(&self) -> impl Future<Output = Result<PersistSnapshot, PersistError>> + Send;
    fn save(&self, snapshot: &PersistSnapshot)
        -> impl Future<Output = Result<(), PersistError>> + Send;
}
```

`PersistHandle` is the drop-guard returned by `persist_with`. Holding it keeps the `CacheMutation` observation (and thus the debounced save loop) alive; **dropping it drops the `Subscription`**, so no new saves are scheduled. A save already parked on its debounce timer is detached and may still complete one final save. `PersistHandle::empty()` constructs a no-op handle (tests).

---

## persist_with + PersistOptions

```rust
impl QueryClient {
    pub fn persist_with<P: Persister>(
        &self,
        persister: P,                    // wrapped in Arc<P> internally
        opts: PersistOptions,
        cx: &mut gpui::App,
    ) -> PersistHandle;
}
```

| `PersistOptions` field | type | default |
|---|---|---|
| `filter` | `PersistFilter` | `PersistFilter::All` |
| `max_age` | `std::time::Duration` | `Duration::from_secs(24 * 60 * 60)` (24h); entries older than this at save time are skipped |
| `debounce` | `std::time::Duration` | `Duration::from_millis(500)`; `Duration::ZERO` disables the timer window (saves still serialize through the drain slot) |

```rust
#[derive(Clone, Debug)]
pub enum PersistFilter {       // owned counterpart to core's borrowing QueryKeyFilter<'a>
    Exact(QueryKey),           // only this key
    Prefix(QueryKey),          // every key that starts with this prefix
    All,                       // every persistable entry
}
impl PersistFilter { pub fn matches(&self, key: &QueryKey) -> bool; }
```

The observer callback collects a snapshot on the main thread (cheap; has `&App`), stashes it in a shared `Mutex<Option<PersistSnapshot>>` slot (replacing any pending one), then spawns a debounced task on the `background_executor` that, after `opts.debounce`, drains the slot and runs `persister.save`. An `armed` flag bounds in-flight tasks to one per window — a bump arriving while a task is already armed skips spawning (its snapshot still lands in the slot, drained by the armed task). Latest snapshot wins.

---

## hydrate + the registries

```rust
pub async fn hydrate<P: Persister>(
    client: &mut QueryClient,
    persister: &P,
    filter: &PersistFilter,
    max_age: Duration,
    cx: &mut gpui::App,
) -> Result<PersistSnapshot, PersistError>;
```

Loads the snapshot, double-checks `version == PERSIST_VERSION` (returns `VersionMismatch` otherwise), then for **every** registered deserializer walks **every** entry and lets the step decode + prime it. Returns the post-filter snapshot so you can do additional metadata-only priming or diagnostics.

> Name collision: there is also a legacy `QueryClient::hydrate(&mut self, _state, _cx)` **method** (metadata-only, no-op stub — see Legacy tier below). The value-carrying primitive is the **free function** `hydrate(...)`.

```rust
impl QueryClient {
    pub fn register_serializer<T, E>(&mut self, f: fn(&T) -> serde_json::Value)
    where T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static;

    pub fn register_deserializer<T, E>(&mut self, deserialize: fn(&serde_json::Value) -> Option<T>)
    where T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static;

    pub fn collect_persist_snapshot(
        &self, filter: &PersistFilter, max_age: Duration, cx: &gpui::App,
    ) -> PersistSnapshot;
}
```

- `f`/`deserialize` are **plain `fn` pointers** (not closures) — `Send + Sync + 'static` with no boxing at the call site. The registry boxes each pointer internally (`Box<dyn Fn>` / `Arc<dyn Fn>`) for type-erased storage — one allocation per registered type, not per save.
- `SerializerRegistry` keys on `TypeId::of::<T>()` alone (not `(T, E)`), because the bucket impls look up by `T`. Serialization depends only on the data type. **Registering two serializers for the same `T` under different `E` silently overwrites** (last wins); whichever survives applies to both `(T, E)` buckets, which is correct because the value *is* that `T`.
- `DeserializerRegistry` is a `Vec<(TypeId, step)>`. **`hydrate` offers each entry to every deserializer** — O(deserializers × entries). There is no type discriminator on `PersistedEntry`, so routing is by trial.
- **Strict-deserializer contract:** a deserializer MUST return `None` for any JSON shape it does not recognize as its own `T`. A lax decoder that accepts a foreign shape wastes work and can mis-prime. Keep them strict and cheap (`v.as_str().map(...)` for a string, `serde_json::from_value(v).ok()` for a struct).

---

## FilePersister (gpui-query-persist)

```rust
pub enum PersistFormat { Json, Bincode }   // Json default; Bincode smaller/faster, not human-readable

pub struct FilePersister { /* path, format, write_lock: Mutex<()> */ }

impl FilePersister {
    pub fn new(path: impl Into<PathBuf>, format: PersistFormat) -> Self;
    pub fn json(path: impl Into<PathBuf>) -> Self;                       // = new(_, Json)
    pub fn bincode(path: impl Into<PathBuf>) -> Self;                    // = new(_, Bincode)
    pub fn in_cache_dir(app_name: impl AsRef<str>) -> Result<Self, PersistError>; // dirs::cache_dir/app/gpui-query-cache.json (Json)
    pub fn path(&self) -> &Path;
}
impl Persister for FilePersister { async fn load(&self) -> Result<PersistSnapshot, PersistError>; async fn save(&self, &PersistSnapshot) -> Result<(), PersistError>; }
```

**Atomic durable write** (`save`): ensure parent dir → serialize → write to a sibling `tempfile::NamedTempFile` → `sync_all` (fsync) → on macOS issue `F_FULLFSYNC` (flushes the drive's own cache; best-effort, logged on failure) → `tempfile`'s `.persist()` renames over the target (`rename(2)` POSIX / `MoveFileEx` Windows) → on POSIX, fsync the **parent directory** so the rename is durable across power loss. Writes serialize through an internal `Mutex`, so concurrent `save` calls from the background executor never interleave temp-file lifecycles.

**Tolerant load** (`load`):

| On-disk state | Result |
|---|---|
| Missing file | empty snapshot (no error) |
| Corrupt / unparseable | logged via `eprintln!` + **empty snapshot** (no panic) |
| `version != PERSIST_VERSION` | `Err(PersistError::VersionMismatch { expected, found })` — typed, so you can distinguish corrupt from wrong-format |
| Valid | decoded snapshot |

**Error mapping:** Windows `ERROR_ACCESS_DENIED` during the atomic replace (antivirus / concurrent reader) maps to `PersistError::Permission` (retryable — back off and retry). Every other IO failure is `PersistError::Io` with the original `std::io::Error` (kind + source chain intact).

`save`/`load` do **synchronous `std::fs` I/O** in their async bodies — intended for GPUI's `background_executor` (a blocking-friendly pool). On a tokio multi-thread runtime, wrap in `spawn_blocking`. **Bincode format** JSON-encodes each entry's `value` to a `String` inside a bincode-safe adapter (bincode can't drive `serde_json::Value`'s `deserialize_any`); the conversion is lossless.

`pub use gpui_query::client::NoopPersister;` is re-exported from this crate as a one-stop default/test persister.

---

## Full cold-start example

```rust
use std::time::Duration;
use gpui::{App, AppContext as _, Global};
use gpui_query::client::{
    hydrate, PersistFilter, PersistOptions, QueryClient,
};
use gpui_query::core::{CachePolicy, Fetched, QueryError, QueryKey, RequestPolicy};
use gpui_query::hook::{use_query_with_policy, QueryOptions};
use gpui_query_persist::FilePersister;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct User { id: u64, name: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Users(Vec<User>);

// Global so the bootstrap task can install the client before any view reads it.
struct ClientGlobal(QueryClient);
impl Global for ClientGlobal;

fn bootstrap(cx: &mut App) {
    let mut client = QueryClient::new();

    // 1. Register a (de)serializer for each T you want to round-trip.
    client.register_serializer::<Users, QueryError>(|u| serde_json::to_value(u).unwrap());
    client.register_deserializer::<Users, QueryError>(|v| serde_json::from_value(v.clone()).ok());

    let persister = FilePersister::json("/var/cache/myapp/gpui-query-cache.json");
    let max_age = Duration::from_secs(60 * 60 * 24);
    let filter = PersistFilter::All;

    // 2. Cold start: hydrate primes the live cache from disk. Block on it from
    //    a background task; GPUI's background_executor is blocking-friendly.
    cx.background_executor().spawn({
        let persister = persister; // FilePersister is Send + Sync + Clone-free
        async move {
            // NOTE: hydrate needs &mut QueryClient + &mut App — drive it in a
            // cx.update_global lease, not detached across an await holding a guard.
        }
    }).detach();

    cx.set_global(ClientGlobal(client));

    // (Hydrate must run inside a global lease that owns &mut App. The typical
    // shape is a blocking `cx.run` / `block_on` for the ready load, or a
    // spawn that re-enters via update_global. See the hydrate signature above.)

    // 3. Install the debounced save driver. Hold the handle for the app lifetime.
    let _handle = cx.update_global::<ClientGlobal, _>(|ClientGlobal(client), cx| {
        client.persist_with(
            persister,
            PersistOptions { filter, max_age, ..PersistOptions::default() },
            cx,
        )
    });
}
```

Then a consuming component fetches as usual; a real fetch completion bumps `CacheMutation`, which the driver observes and saves:

```rust
fn render_users(cx: &mut gpui::Context<impl 'static>) {
    let (entity, _sub) = use_query_with_policy::<Users, QueryError, _, _, _>(
        QueryOptions::new(["users", "all"])
            .cache_policy(CachePolicy::Ttl { ttl_ms: 60_000 })
            .request_policy(RequestPolicy::LatestWins),
        |signal| async move {
            let resp: Vec<User> = fetch_users().await?;          // your async
            if signal.is_cancelled() { return Err(QueryError::cancelled("aborted")); }
            Ok(Fetched::new(Users(resp)))
        },
        cx,
    );
    // entity.read(cx).data() / .status() ...
}
# async fn fetch_users() -> Result<Vec<User>, QueryError> { Ok(vec![]) }
```

Both `use_query` completions and `use_mutation` / `use_infinite_query` first-page completions bump the dirty signal; the imperative `prepare_fetch_query().complete_success(...)` path bumps it too, so nothing fetched through the public API is invisible to the driver.

---

## HTTP cache mental model

```
response headers ──► cache_policy_from_headers() ──► CachePolicy ──► Fetched::with_policy()
                                              │
                       HttpCache<B> in-memory layer over HttpBackend
                       (fresh short-circuit, conditional GET, 304 re-serve)
```

Two independent pieces:

1. **`cache_policy_from_headers`** — pure header → `CachePolicy` ("server wins"). Hand the result to `Fetched::with_policy(data, policy)` so the resource adopts the server's TTL.
2. **`HttpCache<B>`** — a URL-keyed in-memory layer over any `HttpBackend`. Fresh entries short-circuit the network; stale entries revalidate with `If-None-Match` / `If-Modified-Since`; a `304` re-serves the cached body without transferring a new one. This is the cheap-revalidation cache that lives *inside* your fetcher, orthogonal to gpui-query's own resource cache.

`CacheMeta` is `Serialize + Deserialize` so a future persistence layer can store it alongside the body and rehydrate a cold start with valid `ETag`s — enabling cheap `304` refetches on the first request after launch. It uses `SystemTime` (epoch-relative, serde-supported), **never** `Instant` (no serde, meaningless across restarts).

---

## cache_policy_from_headers rules

```rust
pub fn cache_policy_from_headers(headers: &http::HeaderMap)
    -> Result<CachePolicy, ParseError>;
```

Priority order (from [RFC 9111]):

1. **`no-store` / `no-cache`** (any value, including bare) → `CachePolicy::NoCache`. Short-circuits immediately — a malformed trailing directive (e.g. `no-store, max-age=abc`) does NOT surface a parse error.
2. **`s-maxage=N`** (shared-cache directive) takes precedence over `max-age=N` when both present. The chosen TTL yields `CachePolicy::Ttl { ttl_ms: N*1000 }`. If `stale-while-revalidate=M` is also present, yields `CachePolicy::StaleWhileRevalidate { ttl_ms, stale_ms: M*1000 }` instead.
3. **Otherwise** → `CachePolicy::NoCache`. There is **no `Expires`-based heuristic** (reserved for a later addition).

Directive names are matched **case-insensitively**; values may be quoted (`max-age="600"`). Multiple `Cache-Control` headers combine. Other directives (`public`, `private`, …) are ignored unless they map to a rule above.

`ParseError { InvalidMaxAge(String), InvalidStaleWhileRevalidate(String) }` — only a malformed TTL value (e.g. `max-age=abc`) produces an error; wrap with `.unwrap_or(CachePolicy::NoCache)` to degrade gracefully.

[RFC 9111]: https://www.rfc-editor.org/rfc/rfc9111

```rust
let policy = cache_policy_from_headers(&resp.headers).unwrap_or(CachePolicy::NoCache);
let fetched = Fetched::with_policy(data, policy);
```

---

## HttpBackend + HttpCache flow

```rust
pub struct Conditionals {
    pub if_none_match: Option<String>,      // from a cached ETag
    pub if_modified_since: Option<String>,  // from a cached Last-Modified
}
impl Conditionals { pub fn from_meta(meta: Option<&CacheMeta>) -> Self; }

pub struct BackendResponse {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub body: bytes::Bytes,                 // owned, outlives the connection
}

pub trait HttpBackend: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;
    fn fetch(&self, url: &str, conditionals: Conditionals)
        -> impl Future<Output = Result<BackendResponse, Self::Error>> + Send;
}
```

The trait uses `-> impl Future + Send` (not `async fn`) so the future is guaranteed `Send` for any executor. This makes it **non-object-safe** — dispatch is static via `HttpCache<B: HttpBackend>`, no `dyn`/`Pin<Box<dyn Future>>` overhead.

```rust
pub struct HttpCache<B: HttpBackend> { /* backend + two Mutex<HashMap> */ }

impl<B: HttpBackend> HttpCache<B> {
    pub fn new(backend: B) -> Self;
    pub async fn fetch(&self, url: &str)
        -> Result<(bytes::Bytes, CachePolicy, Option<CacheMeta>), HttpError>;
}
```

`fetch` branches:

| Branch | Behavior |
|---|---|
| **Fresh hit** (`stored_at + fresh_for > now`) | return cached body immediately; **backend never called** |
| Stale / first fetch | build `Conditionals::from_meta(cached)`, call `backend.fetch` |
| **`304 Not Modified`** | re-serve cached body + stored policy/meta; `Err(NotModifiedWithoutCachedBody)` if no cached body exists |
| **`200 OK`** | parse policy via `cache_policy_from_headers`; if `NoCache`, serve body + store nothing; else store body + meta, return fresh triple |
| Any other status | return body + `CachePolicy::NoCache` + `None`; nothing stored |

Only `200`s with a cacheable policy populate the cache. `meta` is `None` only for non-cacheable responses.

```rust
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("backend request failed")]
    Backend { #[source] source: Box<dyn std::error::Error + Send + Sync + 'static> },
    #[error(transparent)]
    InvalidPolicy(#[from] ParseError),
    #[error("received 304 without a cached body for {url:?}")]
    NotModifiedWithoutCachedBody { url: String },
    #[error("cache mutex poisoned")]
    Poisoned,
}
```

**Concurrency:** state is guarded by two `std::sync::Mutex`es (one for meta, one for bodies). A guard is acquired, the needed value is **cloned out, and the guard dropped before any `.await` point** — so the cache never holds a `std` mutex across `.await`. It is `Send + Sync` and requires **no tokio** (works on GPUI's `background_executor`, tokio, or anything else).

`CacheMeta` round-trips through persistence: a non-zero `stale_for` reconstructs `StaleWhileRevalidate`, a non-zero `fresh_for` reconstructs `Ttl`, both-zero collapses to `NoCache` (mirroring `fresh_for_from_policy` / `stale_for_from_policy`).

---

## ReqwestBackend

Behind the `reqwest` cargo feature (`gpui-query-http = { features = ["reqwest"] }`):

```rust
pub struct ReqwestBackend(pub reqwest::Client);

impl ReqwestBackend {
    pub fn from_client(client: reqwest::Client) -> Self;   // for custom TLS/timeouts/proxies
}
impl HttpBackend for ReqwestBackend {
    type Error = reqwest::Error;
    fn fetch(&self, url: &str, conditionals: Conditionals)
        -> impl Future<Output = Result<BackendResponse, reqwest::Error>> + Send;
}
```

The request is built **synchronously** (no `.await`) — `If-None-Match` / `If-Modified-Since` attached when present — then the `send` + `bytes()` half is returned as a `Send` future. This eager build keeps the future `Send` even where `reqwest::RequestBuilder` is `!Send`. The client is reused across requests (configure TLS provider, timeouts, proxies on the `reqwest::Client` before wrapping).

`reqwest` is *one* possible backend. Any client that can do a conditional `GET` and produce status + headers + body can `impl HttpBackend` and feed `HttpCache::new`.

---

## Full HTTP example: HttpCache inside a with_policy fetcher

```rust
use gpui_query::core::{CachePolicy, Fetched, QueryError};
use gpui_query::hook::{use_query_with_policy, QueryOptions};
use gpui_query_http::{HttpCache, ReqwestBackend, cache_policy_from_headers};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Release { tag: String }

// One cache per app, shared across fetchers. Wrap it in an `Arc` — `HttpCache`
// is NOT `Clone` (it holds `Mutex<HashMap>`s), so clone the `Arc`, not the cache.
fn http_cache() -> std::sync::Arc<HttpCache<ReqwestBackend>> {
    std::sync::Arc::new(HttpCache::new(ReqwestBackend::from_client(reqwest::Client::new())))
}

fn fetch_releases(cx: &mut gpui::Context<impl 'static>) {
    let cache = http_cache();
    let (entity, _sub) = use_query_with_policy::<Release, QueryError, _, _, _>(
        QueryOptions::new(["releases", "latest"]),
        move |_signal| {
            let cache = cache.clone();     // cheap Arc clone — one per fetch invocation
            async move {
                // 1. HttpCache.fetch handles fresh short-circuit / 304 revalidation.
                let (body, policy, meta) = cache
                    .fetch("https://api.example.com/releases/latest")
                    .await
                    .map_err(|e| QueryError::transport(format!("http: {e}")))?;

                let release: Release = serde_json::from_slice(&body)
                    .map_err(|e| QueryError::unknown(format!("decode: {e}")))?;

                // 2. Server wins: adopt the response's CachePolicy.
                let mut fetched = Fetched::with_policy(release, policy);

                // 3. (persist feature) Carry CacheMeta so the next cold start
                //    can issue a conditional GET immediately.
                if let Some(m) = meta {
                    fetched = fetched.with_meta(serde_json::to_value(m).unwrap());
                }
                Ok(fetched)
            }
        },
        cx,
    );
    // entity.read(cx) ...
}
```

`Fetched` API:

```rust
impl<T> Fetched<T> {
    pub fn new(data: T) -> Self;                          // keep the caller's policy
    pub fn with_policy(data: T, policy: CachePolicy) -> Self;  // override with server's
    #[cfg(feature = "persist")]
    pub fn with_meta(mut self, meta: serde_json::Value) -> Self; // attach CacheMeta etc.
}
```

`Fetched::with_meta` requires the `persist` feature; the `meta` flows into `PersistedEntry::meta` when the resource is persisted, enabling cold-start revalidation.

---

## Legacy tier (metadata-only) — steer away

Alongside the value-carrying `Persister`, the main crate retains an older **metadata-only** persistence API (also gated behind `persist`), kept for back-compat. It carries **no data**:

```rust
#[cfg(feature = "persist")]
pub trait QueryPersister: Send + Sync {
    fn load(&self) -> Vec<DehydratedEntry>;
    fn save(&self, entries: Vec<DehydratedEntry>);
}

#[cfg(feature = "persist")]
pub struct DehydratedEntry { pub key: String, pub type_id: std::any::TypeId, pub kind: &'static str }
```

And on `QueryClient`:

| Method | Behavior |
|---|---|
| `dehydrate(&self, cx: &App) -> DehydratedState` | collects key + `TypeId` + kind of `Success` entries only (**no data**) |
| `hydrate(&mut self, _state, _cx)` | **no-op stub** — body is empty; callers must iterate and `set_query_data` themselves |
| `persist(&self, &dyn QueryPersister, cx)` | dehydrates + saves entries (metadata-only) |
| `restore(persister: &dyn QueryPersister) -> Vec<DehydratedEntry>` | associated fn (no `&self`); loads raw entries |

Prefer the **value-carrying** `Persister` + `persist_with` + free-fn `hydrate` for any new code — it round-trips real data through the serializer/deserializer registries. The legacy types exist only to avoid breaking the old `dehydrate`/`hydrate`/`persist`/`restore` surface.

---

## Gotchas

- **Strict deserializers are load-bearing.** `hydrate` offers every on-disk entry to every registered deserializer (O(n × m), no type discriminator). A permissive decoder that accepts a foreign shape will mis-prime the wrong bucket. Return `None` for anything that isn't unambiguously your `T`.
- **TypeId-only registry overwrites.** `register_serializer::<T, E_a>` then `register_serializer::<T, E_b>` for the same `T` silently overwrites — both are keyed on `TypeId::of::<T>()`. Last write wins, and it applies to both `(T, E)` buckets. This is correct (the value *is* that `T`) but surprises people expecting per-`(T, E)` keying.
- **Non-object-safe traits.** `Persister` and `HttpBackend` both return `impl Future + Send` and are consumed generically (`persist_with<P: Persister>`, `HttpCache<B: HttpBackend>`). You cannot `Box<dyn Persister>` or `Box<dyn HttpBackend>` — use an `enum` of backends or generic plumbing instead.
- **Mutex guards never cross `.await`.** Both `HttpCache` and `FilePersister` acquire a `std::sync::Mutex`, clone the value out, and drop the guard before yielding. If you write your own `Persister`/`HttpBackend`, do the same — holding a `std` mutex across `.await` is undefined behavior (the future is `Send` but the guard often is not) and trips on some runtimes.
- **Only `200`s are cached.** A `304` re-serves a *prior* `200` body; a `304` with no cached body is `Err(NotModifiedWithoutCachedBody)`. `no-store`/`no-cache` and non-`200`/`304` statuses store nothing.
- **`no-store` short-circuits parsing.** `no-store, max-age=abc` returns `Ok(NoCache)` — the malformed `max-age` is never reached. Only a malformed TTL *without* a preceding `no-store`/`no-cache` yields `InvalidMaxAge`.
- **macOS Metal Toolchain.** Building `client`/`hook`/`persist` (so, the persist feature and `gpui-query-persist`) fails on macOS without it: run `xcodebuild -downloadComponent MetalToolchain` once. `gpui-query-http` (core-only) is unaffected.
- **`PersistHandle` drop stops *new* saves.** A save already parked on its debounce timer is detached and may still complete once after you drop the handle. Keep the handle for the app lifetime (store it on a long-lived view/entity) if you want continuous persistence.
- **`Debounced saves use GPUI's timer.** In tests, the mock clock does not advance on `run_until_parked`; use `cx.background_executor().advance_clock(debounce + ε)` to mature the timer, or set `debounce: Duration::ZERO` for immediate saves.

---

## Pointers

- Persistence guide: `https://gpui-query.freeoxide.com/docs/guides/persistence`
- HTTP caching guide: `https://gpui-query.freeoxide.com/docs/guides/http-caching`
- Cache policies deep-dive: `https://gpui-query.freeoxide.com/docs/blog/cache-policies-explained`
- Crate docs: `https://docs.rs/gpui-query`, `https://docs.rs/gpui-query-http`, `https://docs.rs/gpui-query-persist`
- RFC 9111 (Cache-Control): `https://www.rfc-editor.org/rfc/rfc9111`

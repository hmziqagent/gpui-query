# gpui-query-http

HTTP cache-header helpers for [gpui-query](https://crates.io/crates/gpui-query). Turn server cache headers into a [`CachePolicy`](https://docs.rs/gpui-query/latest/gpui_query/core/enum.CachePolicy.html) ("server wins") and layer an in-memory `HttpCache` over any HTTP client.

gpui-query's core cache policies (`NoCache`, `Ttl`, `StaleWhileRevalidate`) are caller-supplied. When the resource you are fetching is an HTTP response, the server already knows how long it should live: `Cache-Control: max-age=600, stale-while-revalidate=120`. This crate parses those headers into the right `CachePolicy` and adds a URL-keyed cache that revalidates with conditional `GET`s.

It depends on gpui-query's **`core`** feature only (no GPUI), so it is usable from any async context. `reqwest` is one *optional* backend, never a hard dependency.

## Install

```toml
[dependencies]
gpui-query-http = "0.1.0"
```

The default feature set is library-agnostic: parse headers and bring your own `HttpBackend`. To use the shipped reqwest backend:

```toml
[dependencies]
gpui-query-http = { version = "0.1.0", features = ["reqwest"] }
```

The usage examples also reference `gpui-query` (for `core::{CachePolicy, Fetched}`) and the `http` crate (for `HeaderMap`). Add them alongside if you haven't: `cargo add gpui-query http`.

## What it does

- **Header → policy ("server wins").** `cache_policy_from_headers` reads RFC 9111 `Cache-Control` directives and returns the matching `gpui_query::core::CachePolicy`.
- **In-memory HTTP cache.** `HttpCache<B>` wraps any `HttpBackend`: fresh entries short-circuit the network entirely, stale entries revalidate with `If-None-Match` / `If-Modified-Since`, and a `304 Not Modified` refreshes the entry without transferring a body.
- **Pluggable backend.** `HttpBackend` abstracts a single conditional `GET`. The crate ships `ReqwestBackend` behind the `reqwest` feature; any other client can implement the trait and feed `HttpCache::new`.
- **Serializable metadata.** `CacheMeta` (ETag, `Last-Modified`, `stored_at`, `fresh_for`, `stale_for`) is serde-serializable, so it round-trips through a persistence layer for cheap `304` refetches on cold start.
- **Typed errors.** `ParseError` (`InvalidMaxAge`, `InvalidStaleWhileRevalidate`) for malformed directives; `HttpError` for backend failures, bad policies, poisoned mutexes, and spurious `304`s.

Parsing rules (priority order):

1. `no-store` or `no-cache` → `CachePolicy::NoCache` (short-circuits immediately).
2. `max-age=N` (seconds) → `CachePolicy::Ttl { ttl_ms: N * 1000 }`; add `stale-while-revalidate=M` → `CachePolicy::StaleWhileRevalidate { ttl_ms, stale_ms }`.
3. Otherwise → `CachePolicy::NoCache`.

`s-maxage` takes precedence over `max-age` when both are set. Directive names are matched case-insensitively and values may be quoted (`max-age="600"`).

## Usage

Parse the response headers inside your fetcher and hand the server's policy to `Fetched::with_policy`. The resource adopts the server's TTL ("server wins"):

```rust
use gpui_query_http::cache_policy_from_headers;
use gpui_query::core::{CachePolicy, Fetched};
use http::HeaderMap;

// `headers` comes from your HTTP response; `data` is the decoded body.
fn build_fetched(data: String, headers: &HeaderMap) -> Fetched<String> {
    let policy = cache_policy_from_headers(headers).unwrap_or(CachePolicy::NoCache);
    Fetched::with_policy(data, policy)
}
```

Return that `Fetched<T>` from a fetcher passed to [`gpui_query::use_query_with_policy`](https://docs.rs/gpui-query/latest/gpui_query/hook/fn.use_query_with_policy.html) (enable gpui-query's `hook` feature): the server policy replaces the caller-supplied policy for that resource immediately after a successful fetch.

For the cache layer, wrap any `HttpBackend`. With the `reqwest` feature:

```rust
use gpui_query_http::{HttpCache, ReqwestBackend};

let cache = HttpCache::new(ReqwestBackend::from_client(reqwest::Client::new()));

// Fresh hit → no network call; stale → conditional GET; 304 → cached body.
let (body, policy, meta) = cache.fetch("https://example.test/data").await?;
```

## WebAssembly

The crate compiles for `wasm32-unknown-unknown` with the default feature set and with the `reqwest` feature — the same feature flags work on both targets. On wasm, `ReqwestBackend` runs on reqwest's browser-fetch backend and TLS is the browser's job; the `rustls-tls` feature that the `reqwest` feature enables for native use is inert there, so it is harmless to leave on.

For custom backends, `HttpBackend::fetch` bounds its returned future with `MaybeSend` (exported at the crate root) instead of `Send`. On native targets `MaybeSend` is exactly `Send`, so an existing impl written with `+ Send` keeps compiling unchanged — no migration needed. Use `+ MaybeSend` only when a backend must compile on both native and wasm targets: on `wasm32`, browser-fetch futures (reqwest's included) are inherently `!Send` because they hold JS values, so a `+ Send` bound would not compile there.

```rust
use std::future::Future;
use gpui_query_http::{BackendResponse, Conditionals, HttpBackend, MaybeSend};

struct MyBackend;

impl HttpBackend for MyBackend {
    type Error = MyError;

    fn fetch(
        &self,
        url: &str,
        conditionals: Conditionals,
    ) -> impl Future<Output = Result<BackendResponse, MyError>> + MaybeSend {
        // perform the conditional GET ...
    }
}
```

## Links

- Website: <https://gpui-query.freeoxide.com>
- Docs: <https://docs.rs/gpui-query-http>
- Source: <https://github.com/freeoxide/gpui-query>
- gpui-query: <https://crates.io/crates/gpui-query>
- RFC 9111 (HTTP Caching): <https://www.rfc-editor.org/rfc/rfc9111>

## Author

**hmziqrs**

- Website: <https://hmziq.rs>
- GitHub: <https://github.com/hmziqrs>
- X: <https://x.com/hmziqrs>

## License

MIT. See the [LICENSE](../../LICENSE) file for details.

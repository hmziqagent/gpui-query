# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

> Wasm32 compile support for `core` and `gpui-query-http`, publish fixes for the satellite crates, and real test coverage in CI.

### Added

#### `gpui-query` — wasm32 support for the `core` layer

- The `core` layer builds for `wasm32-unknown-unknown`: on wasm targets `ahash` switches from runtime RNG to `compile-time-rng` internally (its `runtime-rng` default pulls `getrandom`, which cannot compile there), so no consumer configuration is needed. The `client`, `hook`, and `persist` layers stay native-only — they depend on `gpui`, which does not build for `wasm32-unknown-unknown`.
- The wasm support boundary is documented in the root and main-crate READMEs.

#### `gpui-query-http` — wasm32 support, including the `reqwest` feature

- The crate compiles for `wasm32-unknown-unknown` with the default feature set and with the `reqwest` feature — the same feature flags work on both targets. On wasm, `ReqwestBackend` runs on reqwest's browser-fetch backend and TLS is the browser's job.
- `MaybeSend` marker alias for `Send`, relaxed to a no-op on `wasm32` (exported at the crate root): `HttpBackend::fetch` bounds its returned future with `MaybeSend` instead of `Send`. On native targets the bound is exactly `Send`, so existing `+ Send` backend impls keep compiling unchanged; on `wasm32` it drops the requirement, because browser-fetch futures (reqwest's included) are inherently `!Send` — they hold JS values.
- The README gained a WebAssembly section showing how to write a backend that compiles on both native and wasm targets.

#### CI — wasm compile guard

- New "Wasm Check" workflow (plus a `just wasm-check` recipe mirroring it) builds the core-only main crate and the `http` satellite — with and without `reqwest` — for `wasm32-unknown-unknown`, then the native all-features build, on every push and pull request, so the wasm boundary cannot silently regress.

### Changed

- CI now runs `cargo test --all-features` on every push and pull request (new "Cargo Test" workflow). Previously every workflow was build-only and the full suite ran only locally via `just test`; the job installs the X11 link dependencies (`libx11-xcb-dev`, `libxkbcommon-x11-dev`) that GPUI-linked test binaries need.
- Publish workflows' rust-cache override pointed at a member directory cargo never writes to, so target-dir caching never hit; the override is dropped in favor of the default workspace-root mapping. `gpui-query-legacy` keeps its mapping intentionally — it is excluded from the workspace and is its own workspace root.
- The PR checks workflow now also validates changes to the sibling web deploy workflows (`deploy.yml`, `web-preview.yml`), which previously triggered no checks at all.

### Fixed

#### `gpui-query-http` — publish blocker and docs.rs metadata

- The `gpui-query` dependency now carries `version = "0.2"` alongside its path: `cargo publish` strips path overrides, so the previously versionless dependency made the crate unpublishable.
- docs.rs now renders with every feature and annotates `reqwest`-gated items with the feature that enables them, so `ReqwestBackend` and its module appear with live intra-doc links instead of dead ones.
- Three redundant intra-doc link targets simplified; the rendered docs are unchanged and rustdoc's warnings are gone.

#### `gpui-query-persist` — publish blocker and docs.rs metadata

- Same publish blocker fixed: the `gpui-query` path dependency gains `version = "0.2"`, and `cargo publish --dry-run` now verifies the crate against the crates.io release.
- Fleet-consistent docs.rs metadata added (`all-features = true`, `rustdoc-args = ["--cfg", "docsrs"]`); behaviorally a no-op today, as the crate has no optional features.
- One redundant intra-doc link simplified.

#### `gpui-query` — warning-free core-only builds

- `core`-only builds no longer warn about the unused `last_updated_at_ms` accessor (only the `client` layer reads it).

#### CI — release workflow guards

- Changelog Release now fetches tags on checkout, so the "already released?" guard actually sees existing tags instead of always re-attempting the release.
- The publish and web-deploy jobs run only when the guard decides a release should happen; merging CHANGELOG edits that do not cut a release (such as new `[Unreleased]` entries) no longer re-runs `cargo publish` or redeploys the website.

## [0.2.0] - 2026-07-21

> Disk persistence, server-driven cache policy, and two new companion crates.

### Added

#### Persistence (`gpui-query` with the new `persist` feature)

- `Persister` trait with async `load` / `save`, and the `PersistSnapshot` / `PersistedEntry` / `PersistError` / `PERSIST_VERSION` types that adapters serialize to and from.
- `QueryClient::persist_with(persister, opts, cx) -> PersistHandle`, a debounced driver that coalesces bursts of cache mutations into one snapshot per window (default 500 ms) and writes only entries younger than `max_age` (default 24 h).
- `hydrate(client, persister, filter, max_age, cx)` to restore a cold-start cache from disk, re-checking the persist version before applying entries.
- `PersistOptions` (`filter` / `max_age` / `debounce`) and `PersistFilter` (`Exact` / `Prefix` / `All`) to scope what gets persisted.
- `SerializerRegistry` / `DeserializerRegistry` for round-tripping typed values through `serde_json::Value` without leaking concrete types into the core layer.
- `NoopPersister` for tests and disabled-persistence modes.

#### "Server wins" cache policy (`Fetched`)

- `Fetched<T>` fetcher result wrapper in `core`: return `Result<Fetched<T>, E>` to let a fetcher attach a server-derived `CachePolicy` that overrides the caller's per-query policy on success.
- `Fetched::new` (no override), `Fetched::with_policy` (override), and `Fetched::with_meta` (persist-gated opaque metadata, e.g. an HTTP `CacheMeta` for cheap `304` refetches after relaunch).
- `use_query_with_policy` and `fetch_query_with_policy` hooks that accept `Fetched`-returning fetchers.

#### `gpui-query-http` — HTTP cache-header helpers (new companion crate)

- `cache_policy_from_headers(&HeaderMap)` turns `Cache-Control` into a `CachePolicy` per [RFC 9111]: `no-store` / `no-cache` → `NoCache`, `max-age` / `s-maxage` → `Ttl`, and `stale-while-revalidate` → `StaleWhileRevalidate`. Directive names are matched case-insensitively and values may be quoted.
- `HttpCache<B>` in-memory cache layer, generic over an `HttpBackend` trait so any request library can plug in.
- `BackendResponse`, `Conditionals` (ETag / `If-Modified-Since`), and a serializable `CacheMeta` for persistence round-trip.
- Optional `ReqwestBackend` behind the `reqwest` cargo feature; `reqwest` is never a hard dependency.
- Depends on `gpui-query` `core` only — no GPUI — keeping `http` / `bytes` / `reqwest` out of the core crate.

#### `gpui-query-persist` — disk persistence adapter (new companion crate)

- `FilePersister`, an atomic, durable `Persister`: each save writes to a sibling temp file, fsyncs it (issuing `F_FULLFSYNC` on macOS for true durability), renames it over the target, then fsyncs the parent directory on POSIX so a crash mid-write never corrupts the cache.
- Tolerant load: missing file → empty snapshot; corrupt JSON/bincode → logged warning + empty snapshot (no panic); version mismatch → typed `PersistError::VersionMismatch`.
- `PersistFormat::Json` (default, inspectable) and `PersistFormat::Bincode` (compact), plus `FilePersister::json` / `::bincode` / `::in_cache_dir` constructors.
- `PersistError` surfaces retryable Windows `ERROR_ACCESS_DENIED` (antivirus / concurrent reader) as a distinct `Permission` variant so callers can back off, while preserving the original `io::Error` chain elsewhere.

### Changed

- New `persist` cargo feature gates the persistence module (and the `serde_json` / `thiserror` deps); `core` and `client` stay free of it.
- The workspace gains two crates: `gpui-query-http` (core-only) and `gpui-query-persist` (pulls `persist` + `client` + `hook`).
- Error sanitization now strips connection strings, tokens, paths, emails, and hex keys from messages (documented in the main crate README).

## [0.1.4] - 2026-06-17

> Crate metadata and README improvements on crates.io.

### Added

- Author metadata (authors) and an Author section in both crate READMEs, linking the maintainer's website, GitHub, and X.
- readme field on gpui-query-legacy so its README renders on crates.io.

## [0.1.3] - 2026-06-14

> Decoupled the legacy crate and fixed gpui version compatibility.

### Changed

- `gpui-query-legacy` is now fully decoupled from the main crate, with standalone docs, tests, and improved hook error handling, and it publishes independently.

### Added

- A crate-level README for `gpui-query` on crates.io.

### Fixed

- `read_with` calls in the hook module are now source-compatible across gpui versions.

## [0.1.2] - 2025-06-13

> Single-workflow releases and an independent legacy crate.

### Changed

- The CI pipeline now publishes both crates in a single workflow run. The changelog-release workflow handles tag, GitHub Release, publish, and website deploy without needing a separate trigger.
- `gpui-query-legacy` is now a fully independent crate on crates.io. The `legacy` feature flag and re-export were removed from `gpui-query`. If you need the v1 API, add `gpui-query-legacy` to your Cargo.toml directly.
- Legacy crate publish step tolerates "already uploaded" errors so the main crate can still publish when re-running a workflow.

## [0.1.1] - 2025-06-12

> The v2 rewrite became the main crate; v1 lives on as gpui-query-legacy.

### Changed

- The v2 rewrite at `crates/gpui-query-v2` is now the main crate at `crates/gpui-query`. The old v1 code lives at `crates/gpui-query-legacy`.
- Fixed `read_with` calls in the hook module that returned `Result` instead of the raw value when called from `AsyncApp` context. The fix covers 9 call sites in `fetch_retry.rs`, `internals.rs`, and `fetch_runners.rs`.

### Added

- The legacy crate has `#![deprecated]` and a README pointing to v2.
- All 12 documentation pages have real content now. No more "coming soon" stubs.

### Removed

- All `gpui_query_v2` references in source and docs replaced with `gpui_query`.

## [0.1.0] - 2025-06-10

> Initial public release: core query system, client registry, and GPUI hooks.

### Added

#### Core Layer
- `QueryResource<T, E>` reactive async state container with request lifecycle management
- `QueryStatus` enum (Idle, Loading, Success, Failure) for state tracking
- `CachePolicy` with TTL, Stale-While-Revalidate, LatestWins, and IgnoreWhileLoading variants
- `RequestPolicy` for controlling cache-first vs network-first behavior
- `RetryPolicy` with configurable max attempts, delay, and exponential backoff
- `QueryKey` type-safe cache key with string-based identification
- `QueryKeyFilter` glob-based key matching for cache invalidation patterns
- `QuerySignal` cooperative cancellation via `Arc<AtomicBool>` for clean async lifecycle
- `QueryError` / `QueryErrorKind` structured error types
- `MutationResource` and `MutationStatus` for mutation state tracking
- `NetworkMode` enum (Online, Offline, Always) for connectivity-aware fetching
- `RefetchTrigger` for imperative and automatic revalidation
- `RequestId` / `RequestGuard` / `RequestSequencer` for deduplication and race handling
- `MappedQueryResource` / `SelectTransform` for derived/transformed query state
- `InfiniteQueryResource` with bidirectional pagination and page management

#### Client Layer
- `QueryClient` implementing `gpui::Global` application-wide query registry
- Type-partitioned `QueryBucket<T, E>` for ergonomic typed access
- `MutationBucket<V, T, E>` for mutation state management
- `QueryObserver` and `MutationObserver` for reactive subscriptions
- Built-in garbage collection for stale query entries

#### Hook Layer
- `use_query()` declarative data fetching hook for GPUI components
- `use_mutation()` mutation hook with success/error callbacks
- `use_infinite_query()` pagination hook with bidirectional fetching
- `QueryOptions` and `MutationOptions` for per-hook configuration

#### gpui-query-v2 (Experimental Rewrite)
- Options-first API: `use_query(QueryOptions::new("key"), fetcher, cx)`
- Signal-always fetcher signature: `Fn(QuerySignal) -> Fut`
- `QueryError` with `Display` + `Error` impls and `.sanitized()` for security redaction
- `AHashMap` for faster key lookups
- `QueryPersister` trait with `dehydrate()`/`hydrate()` for state persistence
- `PreparedFetch` for imperative one-shot fetches
- Bounded `max_pages` (default 50) for infinite queries
- Mutation garbage collection
- Property-based testing with proptest
- `use_query_select()` for derived query state
- Extracted `fetch_retry` module with configurable retry logic
- Richer devtools: `ClientDiagnostic`, `DehydratedState`, `DehydratedEntry`

### Changed
- Initial public release

[0.2.0]: https://github.com/freeoxide/gpui-query/releases/tag/v0.2.0
[0.1.4]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.4
[0.1.3]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.3
[0.1.2]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.2
[0.1.1]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.1
[0.1.0]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.0

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-10-02

> A campaign pass over the query lifecycle: direct cache writes now reach the UI, GC and eviction stop dropping live entries, request ids survive bucket churn, error redaction covers the connection strings it missed, and persistence gains dirty tracking, ordered saves, and collision-free key paths.

### Fixed

#### `gpui-query` — direct cache writes reach mounted observers

- `set_query_data`, `PreparedFetch::complete`, and `hydrate` mutated resources without `cx.notify()`, so mounted `use_query`/`use_query_select` consumers never re-rendered on the documented optimistic-update and prefetch flows. They notify now, the observer dedup wakes on data-only writes to an already-`Success` entry, and `reset_queries` wakes its observers too.

#### `gpui-query` — GC and eviction keep live entries

- GC aged every resource without a completion timestamp as fully expired, so live never-fetched entries and `set_query_data`/hydrate-primed keys were dropped at the next sweep and a later `resource()` call minted a divergent second entity for the same key. Entries carry an insertion baseline now, and `invalidate`/`reset` refresh it. `evict_oldest` no longer pays a weak-upgrade scan per insert at the 10,000-entry cap (about 900x the normal insert cost before) and evicts dead entries first. Cancelled mutations are GC-stamped.
- After `remove_queries` dropped a bucket entry while a fetch was in flight, the resource's transient sequencer minted fallback ids in the same space as bucket ids, so a stale completion could pass `accept_current_request` and discard the fresh result. Fallback ids draw from a reserved scope now. Under StaleWhileRevalidate + `IgnoreWhileLoading`, revalidation also spawned a duplicate fetcher racing the original for one request id; that case is ignored at the hook, prefetch, and prepared-fetch sites.

#### `gpui-query` — mutation cancel is a terminal transition

- `MutationResource::cancel` clears stale success data beside the `Failure` status, matching what `complete_failure` always documented, and a late `Ok` from the mutation future can no longer overwrite a terminal `Failure`.

#### `gpui-query` — redaction covers the schemes and shapes it missed

- `sanitized()` matched only four exact scheme spellings, so `postgresql://`, `rediss://`, `mongodb+srv://`, `mysql2://`, `amqp://`, and `mssql://` connection strings leaked credentials verbatim; `PATH_NEEDLES` used forward slashes only, so Windows-style paths leaked entirely; and dotted-local-part + dotless-domain emails (`j.smith@intranet`) escaped the email pass. All three are covered, and the connection and email passes are linear on adversarial input (the email path measured ~895 ms at 32 KiB before; output is byte-identical on all prior inputs).

#### `gpui-query` — persistence

- `QueryKey::to_path` was not injective: `["a::", ""]` and `["a", "::"]` both produced `"a::::"`, so two live queries silently overwrote each other in the snapshot and re-hydrated as a key matching neither origin. The path format escapes the separator and `PERSIST_VERSION` is bumped: old snapshots are discarded rather than misread, so expect one cold cache after upgrading if you persist.
- Every debounce flush re-serialized the entire cache on the UI thread regardless of what changed. Flushes are dirty-tracked by a data epoch, unchanged entries reuse their stored payloads, removals are pruned, saves run strictly in order, and `Persister::save` still receives the full accumulated store.

### Changed

- `QueryResource` exposes an additive `data_epoch()`; `use_query_select` compares epochs instead of deep-comparing `T` on every notification and clones only on real change (the compare was ~93% of per-notify cost on a 1.6 MB payload). The `T: PartialEq` hook bound is unchanged.
- Criterion benches (sanitize, key path, request policy, request id, persist round-trip) land as dev-dependency tooling with a recorded baseline for regression gating; nothing ships to consumers.
- The docs match the shipped API: the rollback guides use the real `set_query_data` capture/restore pattern instead of the nonexistent `rollback_query_data`, and the mutations page documents this crate's `MutationResource` instead of the deprecated legacy crate's.

## [0.2.2] - 2026-09-21

> Audit-driven fixes across all three crates: a release-profile compile break, wider secret redaction, retry counters that match their docs, RFC 9111 cache refresh, and a durability fix in the file persister.

### Fixed

#### `gpui-query` — release-profile compile break

- `cargo build --release` with the `hook` feature failed to compile: a release-only fallback in `use_query` called `cx.new` without the `AppContext` trait in scope. Dev-profile builds were unaffected, which is why the test suite never saw it. CI now builds and lints the release profile on every push and pull request.

#### `gpui-query` — wider secret redaction in error text

- The sanitizer now catches the shapes secrets arrive in: underscore-bearing email local parts (`alice@secret_word@corp.com`), `bearer` and `token` values behind any mix of whitespace and `:`/`=` separators including doubled ones (`bearer ==`), digit-bearing TLDs (`alice@corp.c0m`, `a@b.0rg`), and trailing-dot hostnames (`alice@corp.com.`). Redaction only grows — nothing that was redacted before passes through now.

#### `gpui-query` — `retry_count` behaves the same in every hook

- A fetch result discarded by a newer request no longer clobbers the live entry's retry counter, `use_infinite_query` increments it between attempts, and `use_mutation` resets it when a mutation succeeds.

#### `gpui-query-http` — `304` responses refresh stored entries

- Per RFC 9111 §4.3.4, a validated `304` updates the stored response and its timestamp, unless the 304's own `Cache-Control` blocks caching. Previously the timestamp never moved, so a revalidated entry stayed stale forever after. A late 304 also no longer clobbers a concurrent refresh: the update applies only if the stored metadata is unchanged since the fetch began.

#### `gpui-query-http` — parse errors cap echoed header bytes

- `ParseError` values embed at most 512 bytes of the offending header, suffixed with `...[truncated]`, so hostile `Cache-Control` fields cannot balloon error text.

#### `gpui-query-persist` — parent directory fsynced for bare filenames

- `FilePersister::json("cache.json")` produced an empty parent path, so the post-rename directory fsync silently did nothing. Bare and relative paths now resolve their parent correctly, and the raw `fsync` FFI call is replaced by `File::sync_all`.

### Changed

- READMEs are compiled as doctests, which caught every quick-start calling `App::new()` (never a method on gpui 0.2.2) plus a handful of drifted signatures, all now fixed.
- Version literals in the skill packs are synced by the release workflow, alongside the READMEs and the docs install page.

## [0.2.1] - 2026-09-19

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

[0.2.1]: https://github.com/freeoxide/gpui-query/releases/tag/v0.2.1
[0.2.0]: https://github.com/freeoxide/gpui-query/releases/tag/v0.2.0
[0.1.4]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.4
[0.1.3]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.3
[0.1.2]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.2
[0.1.1]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.1
[0.1.0]: https://github.com/freeoxide/gpui-query/releases/tag/v0.1.0

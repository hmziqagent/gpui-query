//! The URL-keyed HTTP cache, [`HttpCache`].
//!
//! [`HttpCache`] wraps any [`crate::backend::HttpBackend`] and adds an in-memory
//! cache keyed by URL string: fresh entries short-circuit the network entirely,
//! stale entries are revalidated with conditional headers (`If-None-Match` /
//! `If-Modified-Since`) and a `304 Not Modified` response re-serves the cached
//! body without transferring a new one.
//!
//! The cache is library-agnostic — it does not name `reqwest` anywhere. `reqwest`
//! is just one optional backend ([`crate::reqwest_backend::ReqwestBackend`]).
//!
//! # Concurrency
//!
//! State is guarded by [`std::sync::Mutex`]es (one for metadata, one for
//! bodies). The cache is `Send + Sync` and never holds a `std` mutex guard
//! across an `.await` point: a guard is acquired, the needed value is cloned
//! out, and the guard is dropped before any backend call yields. This keeps the
//! cache usable from any async runtime (it does not require `tokio`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use gpui_query::core::CachePolicy;
use http::HeaderMap;
use thiserror::Error;

use crate::backend::{BackendResponse, Conditionals, HttpBackend};
use crate::{CacheMeta, ParseError, cache_policy_from_headers};

/// Errors raised by [`HttpCache::fetch`].
#[derive(Debug, Error)]
pub enum HttpError {
    /// The underlying backend (e.g. `reqwest`) failed to perform the request.
    ///
    /// The original error is preserved as the `#[source]` so callers can
    /// downcast or walk the cause chain.
    #[error("backend request failed")]
    Backend {
        /// The source error from the backend.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// The response cache headers could not be parsed into a [`CachePolicy`].
    ///
    /// Wraps the [`ParseError`] produced by [`cache_policy_from_headers`].
    #[error(transparent)]
    InvalidPolicy(#[from] ParseError),
    /// The server returned `304 Not Modified` but the cache held no prior body
    /// for this URL to fall back on.
    ///
    /// A `304` is only meaningful as a revalidation of a cached entry; without
    /// a cached body there is nothing to serve.
    #[error("received 304 without a cached body for {url:?}")]
    NotModifiedWithoutCachedBody {
        /// The URL that produced the spurious `304`.
        url: String,
    },
    /// A cache [`Mutex`] was poisoned by a panicking thread.
    ///
    /// Rather than panicking the caller (the previous `.expect` behavior), the
    /// poison is surfaced as a typed error so a poisoned cache fails one
    /// request instead of taking down the process.
    #[error("cache mutex poisoned")]
    Poisoned,
}

/// A URL-keyed HTTP cache layered over a [`HttpBackend`].
///
/// Generic over the backend (`HttpCache<B: HttpBackend>`) so dispatch is static
/// and there is no `Box<dyn>` overhead. See the [crate docs](crate) for the
/// concurrency model and the backend module for how to plug in a non-`reqwest`
/// client.
pub struct HttpCache<B: HttpBackend> {
    backend: B,
    meta: Mutex<HashMap<String, CacheMeta>>,
    bodies: Mutex<HashMap<String, Bytes>>,
}

impl<B: HttpBackend> HttpCache<B> {
    /// Create a new cache backed by `backend` and starting empty.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            meta: Mutex::new(HashMap::new()),
            bodies: Mutex::new(HashMap::new()),
        }
    }

    /// Fetch `url`, serving a fresh cached entry without any network call when
    /// possible and revalidating otherwise.
    ///
    /// Returns the body bytes, the [`CachePolicy`] currently in effect, and the
    /// [`CacheMeta`] when an entry exists (it is `None` only for non-cacheable
    /// responses, which never populate the cache).
    ///
    /// # Branches
    ///
    /// - **Fresh cache hit** (`stored_at + fresh_for > now`): returns the cached
    ///   body immediately; the backend is never called.
    /// - **`304 Not Modified`**: returns the previously cached body and the
    ///   stored policy/meta (the entry is still considered valid).
    /// - **`200 OK`**: parses the new cache headers, stores the body + meta, and
    ///   returns them along with the freshly-derived policy.
    /// - **Any other status** (including `no-store` responses): returns the body
    ///   with [`CachePolicy::NoCache`] and `None` for meta; nothing is stored.
    pub async fn fetch(
        &self,
        url: &str,
    ) -> Result<(Bytes, CachePolicy, Option<CacheMeta>), HttpError> {
        // (a) Read cached meta, clone out, DROP the guard before any .await.
        let cached_meta = {
            let guard = self.meta.lock().map_err(|_| HttpError::Poisoned)?;
            guard.get(url).cloned()
        };

        // (b) Fresh short-circuit: no backend call at all.
        if let Some(ref meta) = cached_meta
            && meta.stored_at + meta.fresh_for > SystemTime::now()
        {
            let body = {
                let guard = self.bodies.lock().map_err(|_| HttpError::Poisoned)?;
                guard.get(url).cloned()
            };
            if let Some(body) = body {
                let policy = policy_from_meta(meta);
                return Ok((body, policy, Some(meta.clone())));
            }
            // Fall through: meta exists but body was evicted — revalidate.
        }

        // (c) Build conditional headers from the (possibly absent) cached meta.
        let conditionals = Conditionals::from_meta(cached_meta.as_ref());

        // (d) Perform the backend fetch.
        let resp = self
            .backend
            .fetch(url, conditionals)
            .await
            .map_err(|e| HttpError::Backend {
                source: Box::new(e),
            })?;

        // (e) 304 — revalidation succeeded: serve the cached body.
        if resp.status == 304 {
            let body = {
                let guard = self.bodies.lock().map_err(|_| HttpError::Poisoned)?;
                guard.get(url).cloned()
            };
            let Some(body) = body else {
                return Err(HttpError::NotModifiedWithoutCachedBody {
                    url: url.to_string(),
                });
            };
            let policy = cached_meta
                .as_ref()
                .map(policy_from_meta)
                .unwrap_or(CachePolicy::NoCache);
            let meta = cached_meta.clone();
            return Ok((body, policy, meta));
        }

        // (f) 200 — fresh response: parse policy, store body + meta.
        if resp.status == 200 {
            return self.store_fresh(url, resp).await;
        }

        // (g) Any other status: do not cache; return body with NoCache.
        Ok((resp.body, CachePolicy::NoCache, None))
    }

    /// Parse the policy from a `200` response, persist the body and meta, and
    /// return the served triple.
    async fn store_fresh(
        &self,
        url: &str,
        resp: BackendResponse,
    ) -> Result<(Bytes, CachePolicy, Option<CacheMeta>), HttpError> {
        let BackendResponse {
            status: _,
            headers,
            body,
        } = resp;
        let policy = cache_policy_from_headers(&headers)?;

        // NoCache means the server forbade caching: serve the body but store
        // nothing.
        if policy == CachePolicy::NoCache {
            return Ok((body, CachePolicy::NoCache, None));
        }

        let now = SystemTime::now();
        let meta = CacheMeta {
            etag: header_str(&headers, "etag"),
            last_modified: header_str(&headers, "last-modified"),
            stored_at: now,
            fresh_for: fresh_for_from_policy(policy),
            stale_for: stale_for_from_policy(policy),
        };

        // Store under both locks, dropping each guard before yielding.
        {
            let mut bodies = self.bodies.lock().map_err(|_| HttpError::Poisoned)?;
            bodies.insert(url.to_string(), body.clone());
        }
        {
            let mut meta_guard = self.meta.lock().map_err(|_| HttpError::Poisoned)?;
            meta_guard.insert(url.to_string(), meta.clone());
        }

        Ok((body, policy, Some(meta)))
    }
}

/// Read a single header value as an owned [`String`], or `None` if absent or
/// non-ASCII.
fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// `fresh_for` ([`Duration`]) from a policy's TTL window.
fn fresh_for_from_policy(policy: CachePolicy) -> Duration {
    Duration::from_millis(policy.ttl_ms().unwrap_or(0))
}

/// `stale_for` ([`Duration`]) from a policy's SWR window.
fn stale_for_from_policy(policy: CachePolicy) -> Duration {
    Duration::from_millis(policy.stale_ms().unwrap_or(0))
}

/// Reconstruct the [`CachePolicy`] a [`CacheMeta`] was derived from.
///
/// Mirrors [`fresh_for_from_policy`] / [`stale_for_from_policy`]: a non-zero
/// `stale_for` selects [`CachePolicy::StaleWhileRevalidate`], otherwise a
/// non-zero `fresh_for` selects [`CachePolicy::Ttl`], and both-zero collapses
/// to [`CachePolicy::NoCache`].
fn policy_from_meta(meta: &CacheMeta) -> CachePolicy {
    let ttl_ms = u64::try_from(meta.fresh_for.as_millis()).unwrap_or(0);
    let stale_ms = u64::try_from(meta.stale_for.as_millis()).unwrap_or(0);
    if stale_ms > 0 {
        CachePolicy::StaleWhileRevalidate { ttl_ms, stale_ms }
    } else if ttl_ms > 0 {
        CachePolicy::Ttl { ttl_ms }
    } else {
        CachePolicy::NoCache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendResponse, Conditionals, HttpBackend, MaybeSend};
    use bytes::Bytes;
    use http::HeaderMap;
    use std::collections::VecDeque;
    use std::future::Future;

    /// A mock backend that returns canned [`BackendResponse`]s from a FIFO
    /// queue, recording the number of times `fetch` was actually invoked so
    /// tests can assert short-circuit behavior.
    struct MockBackend {
        responses: Mutex<VecDeque<Result<BackendResponse, MockError>>>,
        calls: Mutex<usize>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("mock backend error")]
    struct MockError;

    impl MockBackend {
        fn new(responses: Vec<Result<BackendResponse, MockError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(0),
            }
        }

        fn calls(&self) -> usize {
            *self.calls.lock().unwrap()
        }

        fn remaining(&self) -> usize {
            self.responses.lock().unwrap().len()
        }
    }

    impl HttpBackend for MockBackend {
        type Error = MockError;

        fn fetch(
            &self,
            _url: &str,
            _conditionals: Conditionals,
        ) -> impl Future<Output = Result<BackendResponse, MockError>> + MaybeSend {
            // Count the call and pop the next canned response.
            let next = {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                let mut responses = self.responses.lock().unwrap();
                responses.pop_front()
            };
            async move {
                match next {
                    Some(Ok(r)) => Ok(r),
                    Some(Err(e)) => Err(e),
                    // Queue exhausted — surface a mock error so the test fails loudly.
                    None => Err(MockError),
                }
            }
        }
    }

    fn resp_200(body: &str, cache_control: &str) -> BackendResponse {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CACHE_CONTROL, cache_control.parse().unwrap());
        BackendResponse {
            status: 200,
            headers,
            body: Bytes::copy_from_slice(body.as_bytes()),
        }
    }

    fn resp_304_with_etag(etag: &str) -> BackendResponse {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::ETAG, etag.parse().unwrap());
        BackendResponse {
            status: 304,
            headers,
            body: Bytes::new(),
        }
    }

    /// (a) A `200` with `max-age` stores the body + meta and returns the
    /// server-derived policy.
    #[tokio::test]
    async fn two_hundred_stores_body_and_meta() {
        let backend = MockBackend::new(vec![Ok(resp_200("hello", "max-age=600"))]);
        let cache = HttpCache::new(backend);

        let (body, policy, meta) = cache.fetch("https://example.test/a").await.unwrap();
        assert_eq!(body, Bytes::from_static(b"hello"));
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
        let meta = meta.expect("200 with cacheable policy yields meta");
        assert_eq!(meta.fresh_for, Duration::from_secs(600));
        assert_eq!(meta.stale_for, Duration::ZERO);
    }

    /// (b) A second fetch while the entry is still fresh short-circuits: the
    /// backend is never called for the second response, so the mock queue still
    /// has it.
    #[tokio::test]
    async fn fresh_entry_short_circuits_no_backend_call() {
        let backend = MockBackend::new(vec![
            Ok(resp_200("first", "max-age=600")),
            // This would be returned on a second backend call — which must NOT happen.
            Ok(resp_200("should-not-happen", "max-age=1")),
        ]);
        let cache = HttpCache::new(backend);

        let (body1, policy1, _) = cache.fetch("https://example.test/b").await.unwrap();
        assert_eq!(body1, Bytes::from_static(b"first"));
        assert_eq!(policy1, CachePolicy::Ttl { ttl_ms: 600_000 });

        let (body2, policy2, _) = cache.fetch("https://example.test/b").await.unwrap();
        assert_eq!(
            body2,
            Bytes::from_static(b"first"),
            "fresh hit serves cached body"
        );
        assert_eq!(policy2, CachePolicy::Ttl { ttl_ms: 600_000 });

        // Exactly one backend call happened; the queued second response is untouched.
        assert_eq!(cache.backend.calls(), 1);
        assert_eq!(
            cache.backend.remaining(),
            1,
            "the second canned response must still be queued"
        );
    }

    /// (c) A `304` to a conditional refetch returns the previously cached body.
    #[tokio::test]
    async fn not_modified_returns_cached_body() {
        let backend = MockBackend::new(vec![
            Ok(resp_200("payload", "max-age=0, stale-while-revalidate=60")),
            Ok(resp_304_with_etag("\"v1\"")),
        ]);
        let cache = HttpCache::new(backend);

        let (body1, _, _) = cache.fetch("https://example.test/c").await.unwrap();
        assert_eq!(body1, Bytes::from_static(b"payload"));

        // max-age=0 → not fresh → triggers a conditional refetch → 304.
        let (body2, _, meta2) = cache.fetch("https://example.test/c").await.unwrap();
        assert_eq!(
            body2,
            Bytes::from_static(b"payload"),
            "304 served cached body"
        );
        assert!(meta2.is_some(), "304 still yields cached meta");
    }

    /// (d) A `200` with `no-store` returns [`CachePolicy::NoCache`] and stores
    /// nothing.
    #[tokio::test]
    async fn no_store_returns_no_cache_and_stores_nothing() {
        let backend = MockBackend::new(vec![Ok(resp_200("ephemeral", "no-store"))]);
        let cache = HttpCache::new(backend);

        let (body, policy, meta) = cache.fetch("https://example.test/d").await.unwrap();
        assert_eq!(body, Bytes::from_static(b"ephemeral"));
        assert_eq!(policy, CachePolicy::NoCache);
        assert!(meta.is_none(), "no-store must not produce meta");

        // Nothing stored — meta map empty.
        assert!(
            cache
                .meta
                .lock()
                .unwrap()
                .get("https://example.test/d")
                .is_none()
        );
    }

    /// (e) A `304` with no cached body for the URL surfaces a typed error rather
    /// than serving nothing — a `304` is only meaningful as a revalidation of a
    /// cached entry.
    #[tokio::test]
    async fn not_modified_without_cached_body_is_typed_error() {
        // First fetch returns 304 directly (no prior cache to fall back on).
        let backend = MockBackend::new(vec![Ok(resp_304_with_etag("\"v1\""))]);
        let cache = HttpCache::new(backend);

        let err = cache.fetch("https://example.test/e").await.unwrap_err();
        assert!(
            matches!(err, HttpError::NotModifiedWithoutCachedBody { .. }),
            "a 304 with no cached body should surface NotModifiedWithoutCachedBody, got {err:?}"
        );
    }
}

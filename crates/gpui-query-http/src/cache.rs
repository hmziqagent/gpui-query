//! The URL-keyed HTTP cache, [`HttpCache`]: fresh entries skip the network,
//! stale entries revalidate with `If-None-Match` / `If-Modified-Since`, and a
//! `304` re-serves the cached body.

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
    /// The backend request failed; the source is kept for cause chains.
    #[error("backend request failed")]
    Backend {
        /// The underlying backend error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// Unparseable cache headers; [`HttpCache::fetch`] degrades them to
    /// [`CachePolicy::NoCache`], so only direct parser callers see this.
    #[error(transparent)]
    InvalidPolicy(#[from] ParseError),
    /// `304 Not Modified` arrived with no cached body to fall back on.
    #[error("received 304 without a cached body for {url:?}")]
    NotModifiedWithoutCachedBody {
        /// The URL that produced the spurious `304`.
        url: String,
    },
    /// A cache mutex was poisoned (fails the request instead of panicking).
    #[error("cache mutex poisoned")]
    Poisoned,
}

/// A URL-keyed cache over a [`HttpBackend`]: exact-string keys (no
/// normalization, `Vary` ignored), no eviction, mutexes never held across `.await`.
pub struct HttpCache<B: HttpBackend> {
    backend: B,
    meta: Mutex<HashMap<String, CacheMeta>>,
    bodies: Mutex<HashMap<String, Bytes>>,
}

impl<B: HttpBackend> HttpCache<B> {
    /// Creates an empty cache over `backend`.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            meta: Mutex::new(HashMap::new()),
            bodies: Mutex::new(HashMap::new()),
        }
    }

    /// Fetches `url`: a fresh entry skips the network, a stale one
    /// revalidates. Returns `(body, policy, meta)`: only a cacheable `200`
    /// stores and yields `meta`, a `304` re-serves the cached body and
    /// refreshes the stored entry unless its own `Cache-Control` blocks
    /// caching or a concurrent fetch replaced the entry, and everything else
    /// is [`CachePolicy::NoCache`] with `None`.
    pub async fn fetch(
        &self,
        url: &str,
    ) -> Result<(Bytes, CachePolicy, Option<CacheMeta>), HttpError> {
        let cached_meta = {
            let guard = self.meta.lock().map_err(|_| HttpError::Poisoned)?;
            guard.get(url).cloned()
        };

        // checked_add: an extreme serde-hydrated stored_at must not panic.
        if let Some(meta) = cached_meta.as_ref()
            && meta
                .stored_at
                .checked_add(meta.fresh_for)
                .is_none_or(|t| t > SystemTime::now())
            && let Some(body) = self.cached_body(url)?
        {
            let policy = policy_from_meta(meta);
            return Ok((body, policy, cached_meta));
        }

        let conditionals = Conditionals::from_meta(cached_meta.as_ref());
        let resp = self
            .backend
            .fetch(url, conditionals)
            .await
            .map_err(|e| HttpError::Backend {
                source: Box::new(e),
            })?;

        if resp.status == 304 {
            let Some(body) = self.cached_body(url)? else {
                return Err(HttpError::NotModifiedWithoutCachedBody {
                    url: url.to_string(),
                });
            };
            if let Some(old) = cached_meta.as_ref()
                && let Some(meta) = refreshed_meta(&resp.headers, old)
            {
                let mut guard = self.meta.lock().map_err(|_| HttpError::Poisoned)?;
                // Lost-update guard: the entry can be replaced between the
                // pre-await clone and this insert; only refresh what was validated.
                if guard
                    .get(url)
                    .is_some_and(|current| same_meta(current, old))
                {
                    guard.insert(url.to_string(), meta.clone());
                    return Ok((body, policy_from_meta(&meta), Some(meta)));
                }
            }
            let policy = cached_meta
                .as_ref()
                .map_or(CachePolicy::NoCache, policy_from_meta);
            return Ok((body, policy, cached_meta));
        }

        if resp.status == 200 {
            return self.store_fresh(url, resp);
        }

        Ok((resp.body, CachePolicy::NoCache, None))
    }

    fn cached_body(&self, url: &str) -> Result<Option<Bytes>, HttpError> {
        let guard = self.bodies.lock().map_err(|_| HttpError::Poisoned)?;
        Ok(guard.get(url).cloned())
    }

    fn store_fresh(
        &self,
        url: &str,
        resp: BackendResponse,
    ) -> Result<(Bytes, CachePolicy, Option<CacheMeta>), HttpError> {
        let BackendResponse { headers, body, .. } = resp;
        let Ok(policy) = cache_policy_from_headers(&headers) else {
            return Ok((body, CachePolicy::NoCache, None));
        };

        if policy == CachePolicy::NoCache {
            return Ok((body, CachePolicy::NoCache, None));
        }

        let meta = CacheMeta {
            etag: header_str(&headers, "etag"),
            last_modified: header_str(&headers, "last-modified"),
            stored_at: SystemTime::now(),
            fresh_for: fresh_for_from_policy(policy),
            stale_for: stale_for_from_policy(policy),
        };

        // Scoped blocks: neither guard is held while taking the other.
        {
            let mut guard = self.meta.lock().map_err(|_| HttpError::Poisoned)?;
            guard.insert(url.to_string(), meta.clone());
        }
        {
            let mut guard = self.bodies.lock().map_err(|_| HttpError::Poisoned)?;
            guard.insert(url.to_string(), body.clone());
        }

        Ok((body, policy, Some(meta)))
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn fresh_for_from_policy(policy: CachePolicy) -> Duration {
    Duration::from_millis(policy.ttl_ms().unwrap_or(0))
}

fn stale_for_from_policy(policy: CachePolicy) -> Duration {
    Duration::from_millis(policy.stale_ms().unwrap_or(0))
}

fn same_meta(a: &CacheMeta, b: &CacheMeta) -> bool {
    a.etag == b.etag
        && a.last_modified == b.last_modified
        && a.stored_at == b.stored_at
        && a.fresh_for == b.fresh_for
        && a.stale_for == b.stale_for
}

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

/// RFC 9111 §4.3.4: a `304` updates stored fields and restarts freshness;
/// a `no-store`/`no-cache`/malformed `Cache-Control` on it leaves the entry
/// untouched.
fn refreshed_meta(headers: &HeaderMap, old: &CacheMeta) -> Option<CacheMeta> {
    let policy = if headers.contains_key(http::header::CACHE_CONTROL) {
        match cache_policy_from_headers(headers) {
            Ok(p) if p != CachePolicy::NoCache => Some(p),
            _ => return None,
        }
    } else {
        None
    };
    Some(CacheMeta {
        etag: header_str(headers, "etag").or_else(|| old.etag.clone()),
        last_modified: header_str(headers, "last-modified").or_else(|| old.last_modified.clone()),
        stored_at: SystemTime::now(),
        fresh_for: policy.map_or(old.fresh_for, fresh_for_from_policy),
        stale_for: policy.map_or(old.stale_for, stale_for_from_policy),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendResponse, Conditionals, HttpBackend, MaybeSend};
    use bytes::Bytes;
    use http::HeaderMap;
    use std::collections::VecDeque;
    use std::future::Future;

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
            let next = {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                self.responses.lock().unwrap().pop_front()
            };
            async move {
                match next {
                    Some(Ok(r)) => Ok(r),
                    Some(Err(e)) => Err(e),
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

    fn resp_200_with_etag(body: &str, cache_control: &str, etag: &str) -> BackendResponse {
        let mut headers = HeaderMap::new();
        headers.insert(http::header::CACHE_CONTROL, cache_control.parse().unwrap());
        headers.insert(http::header::ETAG, etag.parse().unwrap());
        BackendResponse {
            status: 200,
            headers,
            body: Bytes::copy_from_slice(body.as_bytes()),
        }
    }

    enum Step {
        Gated(tokio::sync::oneshot::Receiver<BackendResponse>),
        Ready(BackendResponse),
    }

    struct InterleavedBackend {
        steps: Mutex<VecDeque<Step>>,
        started: tokio::sync::mpsc::UnboundedSender<String>,
    }

    impl HttpBackend for InterleavedBackend {
        type Error = MockError;

        fn fetch(
            &self,
            url: &str,
            _conditionals: Conditionals,
        ) -> impl Future<Output = Result<BackendResponse, MockError>> + MaybeSend {
            let step = self.steps.lock().unwrap().pop_front();
            let _ = self.started.send(url.to_string());
            async move {
                match step {
                    Some(Step::Ready(resp)) => Ok(resp),
                    Some(Step::Gated(rx)) => rx.await.map_err(|_| MockError),
                    None => Err(MockError),
                }
            }
        }
    }

    fn resp_304(cache_control: Option<&str>, etag: Option<&str>) -> BackendResponse {
        let mut headers = HeaderMap::new();
        if let Some(cache_control) = cache_control {
            headers.insert(http::header::CACHE_CONTROL, cache_control.parse().unwrap());
        }
        if let Some(etag) = etag {
            headers.insert(http::header::ETAG, etag.parse().unwrap());
        }
        BackendResponse {
            status: 304,
            headers,
            body: Bytes::new(),
        }
    }

    fn seed_entry<B: HttpBackend>(
        cache: &HttpCache<B>,
        url: &str,
        body: &'static [u8],
        fresh_for: Duration,
        stored_at: SystemTime,
    ) {
        cache.meta.lock().unwrap().insert(
            url.to_string(),
            CacheMeta {
                etag: Some("\"v0\"".to_string()),
                last_modified: None,
                stored_at,
                fresh_for,
                stale_for: Duration::ZERO,
            },
        );
        cache
            .bodies
            .lock()
            .unwrap()
            .insert(url.to_string(), Bytes::copy_from_slice(body));
    }

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

    #[tokio::test]
    async fn fresh_entry_short_circuits_no_backend_call() {
        let backend = MockBackend::new(vec![
            Ok(resp_200("first", "max-age=600")),
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

        assert_eq!(cache.backend.calls(), 1);
        assert_eq!(
            cache.backend.remaining(),
            1,
            "second canned response untouched"
        );
    }

    #[tokio::test]
    async fn not_modified_returns_cached_body() {
        let backend = MockBackend::new(vec![
            Ok(resp_200("payload", "max-age=0, stale-while-revalidate=60")),
            Ok(resp_304(None, Some("\"v1\""))),
        ]);
        let cache = HttpCache::new(backend);

        let (body1, _, _) = cache.fetch("https://example.test/c").await.unwrap();
        assert_eq!(body1, Bytes::from_static(b"payload"));

        let (body2, _, meta2) = cache.fetch("https://example.test/c").await.unwrap();
        assert_eq!(
            body2,
            Bytes::from_static(b"payload"),
            "304 served cached body"
        );
        assert!(meta2.is_some(), "304 still yields cached meta");
    }

    #[tokio::test]
    async fn no_store_returns_no_cache_and_stores_nothing() {
        let backend = MockBackend::new(vec![Ok(resp_200("ephemeral", "no-store"))]);
        let cache = HttpCache::new(backend);

        let (body, policy, meta) = cache.fetch("https://example.test/d").await.unwrap();
        assert_eq!(body, Bytes::from_static(b"ephemeral"));
        assert_eq!(policy, CachePolicy::NoCache);
        assert!(meta.is_none(), "no-store must not produce meta");
        assert!(
            cache
                .meta
                .lock()
                .unwrap()
                .get("https://example.test/d")
                .is_none()
        );
    }

    #[tokio::test]
    async fn malformed_cache_control_degrades_to_no_cache() {
        let backend = MockBackend::new(vec![Ok(resp_200("body", "max-age=abc"))]);
        let cache = HttpCache::new(backend);

        let (body, policy, meta) = cache.fetch("https://example.test/f").await.unwrap();
        assert_eq!(body, Bytes::from_static(b"body"));
        assert_eq!(policy, CachePolicy::NoCache);
        assert!(meta.is_none());
        assert!(
            cache
                .meta
                .lock()
                .unwrap()
                .get("https://example.test/f")
                .is_none()
        );
    }

    #[tokio::test]
    async fn non_two_hundred_is_not_cached() {
        let backend = MockBackend::new(vec![Ok(BackendResponse {
            status: 404,
            headers: HeaderMap::new(),
            body: Bytes::copy_from_slice(b"missing"),
        })]);
        let cache = HttpCache::new(backend);

        let (body, policy, meta) = cache.fetch("https://example.test/g").await.unwrap();
        assert_eq!(body, Bytes::from_static(b"missing"));
        assert_eq!(policy, CachePolicy::NoCache);
        assert!(meta.is_none());
        assert!(
            cache
                .meta
                .lock()
                .unwrap()
                .get("https://example.test/g")
                .is_none()
        );
    }

    #[tokio::test]
    async fn overflow_max_age_caches_saturated() {
        let backend = MockBackend::new(vec![
            Ok(resp_200("big", "max-age=99999999999999999999999")),
            Ok(resp_200("second", "max-age=1")),
        ]);
        let cache = HttpCache::new(backend);

        let (body, policy, _) = cache.fetch("https://example.test/h").await.unwrap();
        assert_eq!(body, Bytes::from_static(b"big"));
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: u64::MAX });

        let (body2, policy2, meta2) = cache.fetch("https://example.test/h").await.unwrap();
        assert_eq!(body2, Bytes::from_static(b"big"));
        assert_eq!(policy2, CachePolicy::Ttl { ttl_ms: u64::MAX });
        assert!(meta2.is_some());
        assert_eq!(cache.backend.calls(), 1);
    }

    #[tokio::test]
    async fn not_modified_without_cached_body_is_typed_error() {
        let backend = MockBackend::new(vec![Ok(resp_304(None, Some("\"v1\"")))]);
        let cache = HttpCache::new(backend);

        let err = cache.fetch("https://example.test/e").await.unwrap_err();
        assert!(
            matches!(err, HttpError::NotModifiedWithoutCachedBody { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn not_modified_restarts_freshness_window() {
        let backend = MockBackend::new(vec![Ok(resp_304(None, Some("\"v1\"")))]);
        let cache = HttpCache::new(backend);
        let url = "https://example.test/refresh";
        seed_entry(
            &cache,
            url,
            b"cached",
            Duration::from_secs(60),
            SystemTime::now() - Duration::from_secs(3600),
        );

        let (body, policy, meta) = cache.fetch(url).await.unwrap();
        assert_eq!(body, Bytes::from_static(b"cached"));
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 60_000 });
        assert_eq!(
            meta.expect("304 yields meta").etag.as_deref(),
            Some("\"v1\""),
            "304 fields update the stored entry"
        );

        let (body2, _, _) = cache.fetch(url).await.unwrap();
        assert_eq!(body2, Bytes::from_static(b"cached"));
        assert_eq!(
            cache.backend.calls(),
            1,
            "refreshed entry serves without the network"
        );
    }

    #[tokio::test]
    async fn not_modified_adopts_new_cache_control() {
        let backend = MockBackend::new(vec![Ok(resp_304(Some("max-age=300"), None))]);
        let cache = HttpCache::new(backend);
        let url = "https://example.test/new-window";
        seed_entry(
            &cache,
            url,
            b"cached",
            Duration::from_secs(60),
            SystemTime::now() - Duration::from_secs(3600),
        );

        let (_, policy, _) = cache.fetch(url).await.unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 300_000 });
        let stored = cache.meta.lock().unwrap().get(url).cloned().unwrap();
        assert_eq!(stored.fresh_for, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn not_modified_with_no_store_keeps_stale_meta() {
        let backend = MockBackend::new(vec![Ok(resp_304(Some("no-store"), None))]);
        let cache = HttpCache::new(backend);
        let url = "https://example.test/no-resurrect";
        let stored_at = SystemTime::now() - Duration::from_secs(3600);
        seed_entry(&cache, url, b"cached", Duration::from_secs(60), stored_at);

        let (body, _, _) = cache.fetch(url).await.unwrap();
        assert_eq!(body, Bytes::from_static(b"cached"));
        let stored = cache.meta.lock().unwrap().get(url).cloned().unwrap();
        assert_eq!(stored.stored_at, stored_at, "no-store 304 must not refresh");
        assert_eq!(stored.etag.as_deref(), Some("\"v0\""));
    }

    #[tokio::test]
    async fn not_modified_with_malformed_cache_control_still_serves() {
        let backend = MockBackend::new(vec![Ok(resp_304(Some("max-age=abc"), None))]);
        let cache = HttpCache::new(backend);
        let url = "https://example.test/malformed-304";
        let stored_at = SystemTime::now() - Duration::from_secs(3600);
        seed_entry(&cache, url, b"cached", Duration::from_secs(60), stored_at);

        let (body, policy, _) = cache.fetch(url).await.unwrap();
        assert_eq!(body, Bytes::from_static(b"cached"));
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 60_000 });
        let stored = cache.meta.lock().unwrap().get(url).cloned().unwrap();
        assert_eq!(stored.stored_at, stored_at);
    }

    #[tokio::test]
    async fn not_modified_refresh_does_not_clobber_concurrent_store() {
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let backend = InterleavedBackend {
            steps: Mutex::new(
                vec![
                    Step::Gated(release_rx),
                    Step::Ready(resp_200_with_etag("new", "max-age=600", "\"v2\"")),
                ]
                .into(),
            ),
            started: started_tx,
        };
        let cache = std::sync::Arc::new(HttpCache::new(backend));
        let url = "https://example.test/lost-update";
        seed_entry(
            &cache,
            url,
            b"old",
            Duration::from_secs(60),
            SystemTime::now() - Duration::from_secs(3600),
        );

        let racing = {
            let cache = std::sync::Arc::clone(&cache);
            tokio::spawn(async move { cache.fetch(url).await })
        };
        assert_eq!(
            started_rx.recv().await.as_deref(),
            Some(url),
            "racing fetch must be parked inside the backend before the writer runs"
        );

        let (_, _, writer_meta) = cache.fetch(url).await.unwrap();
        assert_eq!(
            writer_meta.unwrap().etag.as_deref(),
            Some("\"v2\""),
            "writer fetch stores the newer validator first"
        );

        release_tx.send(resp_304(None, None)).unwrap();
        racing.await.unwrap().unwrap();

        let stored = cache.meta.lock().unwrap().get(url).cloned().unwrap();
        assert_eq!(
            stored.etag.as_deref(),
            Some("\"v2\""),
            "a late 304 must not roll the entry back to the pre-await clone"
        );
        assert_eq!(stored.fresh_for, Duration::from_secs(600));
    }
}

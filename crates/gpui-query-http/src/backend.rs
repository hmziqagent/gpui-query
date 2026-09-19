//! Library-agnostic HTTP backend abstraction.
//!
//! [`HttpBackend`] abstracts a single conditional `GET` so [`crate::HttpCache`]
//! is not hardcoded to one HTTP client. The crate ships an optional
//! `reqwest`-based implementation behind the `reqwest` cargo feature
//! ([`crate::reqwest_backend::ReqwestBackend`]); any other request library can
//! implement this trait instead and feed [`crate::HttpCache::new`].
//!
//! The trait deliberately uses `-> impl Future<Output = …> + MaybeSend`
//! (see [`MaybeSend`]) rather than `async fn` so the returned futures are
//! guaranteed `Send` on native targets and usable from any executor (GPUI's
//! `background_executor`, tokio, etc.); on `wasm32` the bound is a no-op
//! because JS interop types — and therefore `reqwest`'s browser-fetch futures
//! — are inherently `!Send`. This makes the trait non-object-safe; dispatch is
//! generic (`HttpCache<B: HttpBackend>`), which is intentional and avoids
//! `dyn` + `Pin<Box<dyn Future>>` overhead.

use std::future::Future;

use bytes::Bytes;
use http::HeaderMap;

use crate::CacheMeta;

/// Conditional request headers a backend should send on a revalidation fetch.
///
/// Mirrors the two validator headers [`crate::CacheMeta`] tracks. The backend
/// should attach whichever are `Some` to the outgoing request and leave the
/// others unset; servers respond `304 Not Modified` when the validators still
/// match, which [`crate::HttpCache`] turns into a cheap cache hit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conditionals {
    /// The `If-None-Match` header value (sourced from a cached `ETag`).
    pub if_none_match: Option<String>,
    /// The `If-Modified-Since` header value (sourced from a cached
    /// `Last-Modified`).
    pub if_modified_since: Option<String>,
}

impl Conditionals {
    /// Build the conditional headers for a refetch from cached [`CacheMeta`].
    ///
    /// Returns [`Conditionals::default`] (no validators) when `meta` is `None`,
    /// i.e. on a first fetch with nothing cached yet.
    pub fn from_meta(meta: Option<&CacheMeta>) -> Self {
        let Some(meta) = meta else {
            return Self::default();
        };
        Self {
            if_none_match: meta.etag.clone(),
            if_modified_since: meta.last_modified.clone(),
        }
    }
}

/// An owned, library-agnostic HTTP response.
///
/// Backends translate their native response type into this shape so
/// [`crate::HttpCache`] can reason about status, headers and body without
/// depending on any particular client crate. `headers` is an
/// [`http::HeaderMap`] (the de-facto shared header container) and `body` is
/// owned [`bytes::Bytes`] so the response can outlive the underlying client
/// connection.
#[derive(Clone, Debug)]
pub struct BackendResponse {
    /// The HTTP status code (e.g. `200`, `304`).
    pub status: u16,
    /// The response headers, as an [`http::HeaderMap`].
    pub headers: HeaderMap,
    /// The response body, owned.
    pub body: Bytes,
}

/// Marker alias for [`Send`], relaxed to a no-op on `wasm32`.
///
/// [`HttpBackend::fetch`] bounds its returned future by `MaybeSend` so the
/// same trait works everywhere: on native targets the bound is exactly
/// [`Send`] (any executor may move the future across threads), while on
/// `wasm32` browser-fetch backends such as `reqwest`'s are inherently
/// `!Send` (their futures hold JS values) and execution is single-threaded,
/// so no `Send` requirement is imposed.
///
/// Outside `wasm32` this is blanket-implemented: `T: MaybeSend` if and only
/// if `T: Send`.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: ?Sized + Send> MaybeSend for T {}

/// Marker alias for [`Send`], relaxed to a no-op on `wasm32`.
///
/// On `wasm32` every type implements `MaybeSend`: the browser-fetch backend
/// of `reqwest` (and JS interop types generally) is `!Send` by design, and
/// wasm executes on a single thread, so the `Send` requirement is dropped.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// A library-agnostic conditional `GET` backend.
///
/// Implement this for your HTTP client of choice (the crate ships
/// [`crate::reqwest_backend::ReqwestBackend`] behind the `reqwest` feature) and
/// hand an instance to [`crate::HttpCache::new`].
///
/// On native targets the returned future must be `Send` so it can run on any
/// executor; the trait therefore uses `-> impl Future + MaybeSend` (not
/// `async fn`), where [`MaybeSend`] is [`Send`] everywhere except `wasm32`.
/// This makes the trait non-object-safe — dispatch is static, via
/// `HttpCache<B: HttpBackend>`.
///
/// `fetch` must:
///
/// - attach the [`Conditionals`] validator headers (`If-None-Match` /
///   `If-Modified-Since`) to the outgoing request when present,
/// - perform a `GET`,
/// - translate the native response into [`BackendResponse`] (status, headers,
///   body).
pub trait HttpBackend: Send + Sync {
    /// The native error type returned by the underlying client.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Perform a conditional `GET` against `url`.
    ///
    /// The returned future must be [`MaybeSend`] (`Send` on native targets).
    fn fetch(
        &self,
        url: &str,
        conditionals: Conditionals,
    ) -> impl Future<Output = Result<BackendResponse, Self::Error>> + MaybeSend;
}

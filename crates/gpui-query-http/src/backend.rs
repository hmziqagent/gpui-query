//! A library-agnostic conditional `GET` so [`HttpCache`](crate::HttpCache)
//! stays client-agnostic; the crate ships
//! [`ReqwestBackend`](crate::reqwest_backend::ReqwestBackend) behind `reqwest`.

use std::future::Future;

use bytes::Bytes;
use http::HeaderMap;

use crate::CacheMeta;

/// Validators for a revalidation fetch: attach whichever are `Some`, and a
/// server that still matches answers `304`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conditionals {
    /// `If-None-Match` value, from a cached `ETag`.
    pub if_none_match: Option<String>,
    /// `If-Modified-Since` value, from a cached `Last-Modified`.
    pub if_modified_since: Option<String>,
}

impl Conditionals {
    /// Validators from cached `meta`; `None` yields the empty default.
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

/// An owned, client-agnostic response that outlives the underlying connection.
#[derive(Clone, Debug)]
pub struct BackendResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers.
    pub headers: HeaderMap,
    /// The response body.
    pub body: Bytes,
}

/// [`Send`] alias for [`HttpBackend::fetch`]'s future on native targets.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: ?Sized + Send> MaybeSend for T {}

/// No-op on `wasm32`: single-threaded, and JS interop futures are `!Send`.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// A conditional `GET` backend: attach the [`Conditionals`] validators when
/// present and translate the native response into [`BackendResponse`]. The
/// [`MaybeSend`] future makes the trait non-object-safe, so dispatch is
/// static via [`HttpCache<B>`](crate::HttpCache).
pub trait HttpBackend: Send + Sync {
    /// The underlying client's native error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Performs a conditional `GET` against `url`.
    fn fetch(
        &self,
        url: &str,
        conditionals: Conditionals,
    ) -> impl Future<Output = Result<BackendResponse, Self::Error>> + MaybeSend;
}

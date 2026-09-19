//! The optional `reqwest`-based [`HttpBackend`] implementation.
//!
//! This module is only compiled when the `reqwest` cargo feature is enabled:
//!
//! ```toml
//! [dependencies]
//! gpui-query-http = { version = "0.1", features = ["reqwest"] }
//! ```
//!
//! `reqwest` is intentionally *one of many* possible backends — see
//! [`crate::backend`] for the library-agnostic trait. Any HTTP client that can
//! perform a conditional `GET` and produce a status + headers + body can plug
//! into [`crate::HttpCache`] instead.

use std::future::Future;

use crate::backend::{BackendResponse, Conditionals, HttpBackend, MaybeSend};

/// A [`HttpBackend`] backed by [`reqwest`].
///
/// Wraps a `reqwest::Client` (which can be configured with a TLS provider,
/// timeouts, proxies, etc. by the caller before construction). The client is
/// reused across requests, as `reqwest` intends.
#[cfg(feature = "reqwest")]
pub struct ReqwestBackend(pub reqwest::Client);

#[cfg(feature = "reqwest")]
impl ReqwestBackend {
    /// Convenience constructor wrapping the default [`reqwest::Client`].
    ///
    /// For non-default configuration (custom TLS, timeouts, …), construct the
    /// client yourself and pass it to [`ReqwestBackend::from_client`] (or
    /// `ReqwestBackend(client)` directly via the tuple struct).
    pub fn from_client(client: reqwest::Client) -> Self {
        Self(client)
    }
}

#[cfg(feature = "reqwest")]
impl HttpBackend for ReqwestBackend {
    type Error = reqwest::Error;

    fn fetch(
        &self,
        url: &str,
        conditionals: Conditionals,
    ) -> impl Future<Output = Result<BackendResponse, reqwest::Error>> + MaybeSend {
        // Build the request synchronously (no .await), then return the future
        // for the send+collect half. Building eagerly here keeps the returned
        // future `Send` on native targets even though `reqwest::RequestBuilder`
        // itself is `!Send` in some configurations. On wasm32 the future is
        // `!Send` by design (reqwest's browser-fetch backend holds JS values),
        // which the `MaybeSend` bound accommodates.
        let mut req = self.0.get(url);
        if let Some(etag) = conditionals.if_none_match {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(since) = conditionals.if_modified_since {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, since);
        }
        async move {
            let resp = req.send().await?;
            let status = resp.status().as_u16();
            // `reqwest`'s header map re-exports `http::HeaderMap`, so this clone
            // is a cheap reference bump into the owned map.
            let headers = resp.headers().clone();
            let body = resp.bytes().await?;
            Ok(BackendResponse {
                status,
                headers,
                body,
            })
        }
    }
}

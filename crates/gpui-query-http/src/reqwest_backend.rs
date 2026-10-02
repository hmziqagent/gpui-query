//! A [`HttpBackend`] over `reqwest`, enabled via the crate's `reqwest`
//! feature; any client that can perform a conditional `GET` can implement
//! the trait instead.

use std::future::Future;

use crate::backend::{BackendResponse, Conditionals, HttpBackend, MaybeSend};

/// A [`HttpBackend`] over a caller-configured [`reqwest::Client`], reused
/// across requests as `reqwest` intends.
pub struct ReqwestBackend(pub reqwest::Client);

impl ReqwestBackend {
    /// Wraps a pre-configured client; `ReqwestBackend(client)` also works.
    pub fn from_client(client: reqwest::Client) -> Self {
        Self(client)
    }
}

impl HttpBackend for ReqwestBackend {
    type Error = reqwest::Error;

    fn fetch(
        &self,
        url: &str,
        conditionals: Conditionals,
    ) -> impl Future<Output = Result<BackendResponse, reqwest::Error>> + MaybeSend {
        // Build eagerly so the future stays `Send` where RequestBuilder is not.
        let mut req = self.0.get(url);
        if let Some(etag) = conditionals.if_none_match {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(since) = conditionals.if_modified_since {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, since);
        }
        async move {
            let mut resp = req.send().await?;
            let status = resp.status().as_u16();
            let headers = std::mem::take(resp.headers_mut());
            let body = resp.bytes().await?;
            Ok(BackendResponse {
                status,
                headers,
                body,
            })
        }
    }
}

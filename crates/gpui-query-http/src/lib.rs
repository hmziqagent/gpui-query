//! HTTP cache-header helpers for [`gpui_query`] — turn server cache headers into
//! a [`gpui_query::core::CachePolicy`] ("server wins") and layer an in-memory
//! [`HttpCache`] over any [`HttpBackend`].
//!
//! This crate depends on `gpui-query` with the **`core`** feature only (no
//! GPUI), so it is usable from any async context, and it keeps
//! `reqwest` / `http` / `bytes` out of the core crate (Guiding Principle 1 of
//! `docs/features.md`).
//!
//! # Library-agnostic by design
//!
//! [`HttpCache`] is generic over a [`HttpBackend`] — a trait that abstracts a
//! single conditional `GET`. The crate ships *one* optional backend,
//! [`ReqwestBackend`], behind the
//! `reqwest` cargo feature; any other request library can implement
//! [`HttpBackend`] and plug into [`HttpCache::new`](HttpCache::new) instead.
//! `reqwest` is never a hard dependency.
//!
//! # Server wins
//!
//! Parse the response headers with [`cache_policy_from_headers`], hand the
//! resulting [`CachePolicy`] to
//! [`Fetched::with_policy`](gpui_query::core::Fetched::with_policy), and the
//! resource adopts the server's TTL:
//!
//! ```no_run
//! # use gpui_query_http::cache_policy_from_headers;
//! # use gpui_query::core::{CachePolicy, Fetched};
//! # fn handle(response_headers: &http::HeaderMap, data: String) {
//! let policy = cache_policy_from_headers(response_headers)
//!     .unwrap_or(CachePolicy::NoCache);
//! let fetched = Fetched::with_policy(data, policy);
//! # }
//! ```

#![deny(missing_docs)]
// docs.rs renders with `--cfg docsrs` (see [package.metadata.docs.rs]); enable
// `#[doc(cfg(...))]` there so the `reqwest`-gated items are annotated with the
// feature that enables them, matching the main crate's convention.
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::time::Duration;

use gpui_query::core::CachePolicy;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod backend;
pub mod cache;
#[cfg(feature = "reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest")))]
pub mod reqwest_backend;

pub use backend::{BackendResponse, Conditionals, HttpBackend, MaybeSend};
pub use cache::{HttpCache, HttpError};
#[cfg(feature = "reqwest")]
#[cfg_attr(docsrs, doc(cfg(feature = "reqwest")))]
pub use reqwest_backend::ReqwestBackend;

/// HTTP cache metadata extracted from a response.
///
/// Serializable so a future persistence layer can store it alongside the body
/// and rehydrate a cold start with valid ETags, enabling cheap `304` refetches
/// on the first request after launch.
///
/// Timestamps use [`SystemTime`](std::time::SystemTime) (serde-supported,
/// epoch-relative) — never [`std::time::Instant`], which has no serde impl and
/// is meaningless across process restarts. This matches the `current_time_ms()`
/// convention in `gpui-query`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheMeta {
    /// `ETag` response header, if present (for `If-None-Match` on refetch).
    pub etag: Option<String>,
    /// `Last-Modified` response header, if present (for `If-Modified-Since`).
    pub last_modified: Option<String>,
    /// When this cached entry was stored.
    pub stored_at: std::time::SystemTime,
    /// How long the entry is considered fresh (the TTL window).
    pub fresh_for: Duration,
    /// How long a stale entry may be served while revalidating (the SWR window).
    pub stale_for: Duration,
}

/// Errors parsing cache headers into a [`CachePolicy`].
#[derive(Debug, Error)]
pub enum ParseError {
    /// `max-age` (or `s-maxage`) directive had a non-integer value.
    #[error("invalid max-age value: {0}")]
    InvalidMaxAge(String),
    /// `stale-while-revalidate` directive had a non-integer value.
    #[error("invalid stale-while-revalidate value: {0}")]
    InvalidStaleWhileRevalidate(String),
}

/// Derive a [`CachePolicy`] from response cache headers ("server wins").
///
/// Rules, in priority order:
///
/// 1. `Cache-Control: no-store` or `no-cache` (any value, including bare) →
///    [`CachePolicy::NoCache`].
/// 2. `Cache-Control: max-age=N` (seconds) → [`CachePolicy::Ttl`] with
///    `ttl_ms = N * 1000`. If `stale-while-revalidate=M` is also present, yields
///    [`CachePolicy::StaleWhileRevalidate`] instead. `s-maxage` is treated like
///    `max-age` (the shared-cache directive) and takes precedence when both are
///    present.
/// 3. Otherwise → [`CachePolicy::NoCache`] (no usable cache directives; an
///    `Expires`-based heuristic may be added later).
///
/// `max-age` / `s-maxage` take precedence over each other and any other
/// directive per [RFC 9111]. Directive names are matched case-insensitively and
/// values may be quoted (`max-age="600"`).
///
/// [RFC 9111]: https://www.rfc-editor.org/rfc/rfc9111
pub fn cache_policy_from_headers(headers: &HeaderMap) -> Result<CachePolicy, ParseError> {
    let mut s_maxage_secs: Option<u64> = None;
    let mut max_age_secs: Option<u64> = None;
    let mut stale_while_revalidate_secs: Option<u64> = None;

    for value in headers.get_all(http::header::CACHE_CONTROL).iter() {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for directive in raw.split(',') {
            let directive = directive.trim();
            if directive.is_empty() {
                continue;
            }
            let (name, val) = match directive.split_once('=') {
                Some((n, v)) => (n.trim(), Some(v.trim().trim_matches('"'))),
                None => (directive, None),
            };
            match name.to_ascii_lowercase().as_str() {
                // no-store / no-cache win immediately per rule 1 and RFC 9111 §5.2.1.5:
                // they are never cacheable, so a malformed directive that happens to
                // follow them (e.g. `no-store, max-age=abc`) must not surface as a
                // parse error.
                "no-store" | "no-cache" => return Ok(CachePolicy::NoCache),
                "s-maxage" => {
                    if let Some(v) = val {
                        s_maxage_secs = Some(parse_secs(false, v)?);
                    }
                }
                "max-age" => {
                    if let Some(v) = val {
                        max_age_secs = Some(parse_secs(false, v)?);
                    }
                }
                "stale-while-revalidate" => {
                    if let Some(v) = val {
                        stale_while_revalidate_secs = Some(parse_secs(true, v)?);
                    }
                }
                _ => {}
            }
        }
    }

    // s-maxage (shared cache) takes precedence over max-age when both are set.
    let ttl_secs = s_maxage_secs.or(max_age_secs);
    if let Some(secs) = ttl_secs {
        let ttl_ms = secs.saturating_mul(1000);
        return Ok(if let Some(stale_secs) = stale_while_revalidate_secs {
            CachePolicy::StaleWhileRevalidate {
                ttl_ms,
                stale_ms: stale_secs.saturating_mul(1000),
            }
        } else {
            CachePolicy::Ttl { ttl_ms }
        });
    }

    // No usable cache directives — do not cache.
    Ok(CachePolicy::NoCache)
}

/// Parse a `Cache-Control` delta-seconds value into seconds.
///
/// `is_stale` selects which [`ParseError`] variant is returned on a malformed
/// value. HTTP delta-seconds must be non-negative integers; values larger than
/// `u64::MAX` are out of scope (the header itself is bounded far below that).
fn parse_secs(is_stale: bool, raw: &str) -> Result<u64, ParseError> {
    raw.parse::<u64>().map_err(|_| {
        if is_stale {
            ParseError::InvalidStaleWhileRevalidate(raw.to_string())
        } else {
            ParseError::InvalidMaxAge(raw.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_query::core::CachePolicy;
    use http::HeaderMap;

    /// Build a `HeaderMap` from a single `Cache-Control` value.
    fn cc(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(http::header::CACHE_CONTROL, value.parse().unwrap());
        h
    }

    #[test]
    fn max_age_yields_ttl() {
        let policy = cache_policy_from_headers(&cc("max-age=600")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
    }

    #[test]
    fn max_age_and_swr_yields_swr() {
        let policy =
            cache_policy_from_headers(&cc("max-age=600, stale-while-revalidate=120")).unwrap();
        assert_eq!(
            policy,
            CachePolicy::StaleWhileRevalidate {
                ttl_ms: 600_000,
                stale_ms: 120_000
            }
        );
    }

    #[test]
    fn s_maxage_takes_precedence() {
        let policy = cache_policy_from_headers(&cc("max-age=10, s-maxage=30")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 30_000 });
    }

    #[test]
    fn no_store_yields_no_cache() {
        assert_eq!(
            cache_policy_from_headers(&cc("no-store")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn no_cache_directive_yields_no_cache() {
        assert_eq!(
            cache_policy_from_headers(&cc("no-cache")).unwrap(),
            CachePolicy::NoCache
        );
    }

    #[test]
    fn no_store_short_circuits_before_parsing_later_directives() {
        // Rule 1 priority: `no-store` wins immediately, so a malformed trailing
        // `max-age` must NOT surface as `InvalidMaxAge`. (RFC 9111 §5.2.1.5.)
        let policy = cache_policy_from_headers(&cc("no-store, max-age=abc")).unwrap();
        assert_eq!(policy, CachePolicy::NoCache);
    }

    #[test]
    fn no_cache_control_yields_no_cache() {
        let policy = cache_policy_from_headers(&HeaderMap::new()).unwrap();
        assert_eq!(policy, CachePolicy::NoCache);
    }

    #[test]
    fn quoted_value_is_accepted() {
        let policy = cache_policy_from_headers(&cc("max-age=\"600\"")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 600_000 });
    }

    #[test]
    fn directive_names_are_case_insensitive() {
        let policy = cache_policy_from_headers(&cc("Max-Age=60")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 60_000 });
    }

    #[test]
    fn other_directives_are_ignored() {
        // `public`/`private` don't map to a CachePolicy variant; max-age still applies.
        let policy = cache_policy_from_headers(&cc("public, max-age=5")).unwrap();
        assert_eq!(policy, CachePolicy::Ttl { ttl_ms: 5_000 });
    }

    #[test]
    fn multiple_cache_control_headers_combine() {
        let mut h = HeaderMap::new();
        h.insert(http::header::CACHE_CONTROL, "max-age=10".parse().unwrap());
        h.append(
            http::header::CACHE_CONTROL,
            "stale-while-revalidate=20".parse().unwrap(),
        );
        let policy = cache_policy_from_headers(&h).unwrap();
        assert_eq!(
            policy,
            CachePolicy::StaleWhileRevalidate {
                ttl_ms: 10_000,
                stale_ms: 20_000
            }
        );
    }

    #[test]
    fn invalid_max_age_is_typed_error() {
        let err = cache_policy_from_headers(&cc("max-age=abc")).unwrap_err();
        assert!(matches!(err, ParseError::InvalidMaxAge(_)));
        assert!(err.to_string().contains("max-age"));
    }

    #[test]
    fn invalid_swr_is_typed_error() {
        let err =
            cache_policy_from_headers(&cc("max-age=10, stale-while-revalidate=oops")).unwrap_err();
        assert!(matches!(err, ParseError::InvalidStaleWhileRevalidate(_)));
    }

    #[test]
    fn cache_meta_serde_roundtrip() {
        let meta = CacheMeta {
            etag: Some("\"abc\"".to_string()),
            last_modified: None,
            stored_at: std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            fresh_for: Duration::from_secs(60),
            stale_for: Duration::from_secs(120),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: CacheMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.etag, meta.etag);
        assert_eq!(back.last_modified, meta.last_modified);
        assert_eq!(back.fresh_for, meta.fresh_for);
        assert_eq!(back.stale_for, meta.stale_for);
        assert_eq!(back.stored_at, meta.stored_at);
    }
}

mod cache_ops;
mod cache_policy;
mod data_retention;
mod request_interactions;

use crate::core::*;
use crate::tests::test_support::*;

pub(crate) const STORED_AT_MS: u64 = 1_000;

pub(crate) const TTL_MS: u64 = 1_000;

pub(crate) const STALE_MS: u64 = 2_000;

pub(crate) const SWR_TOTAL_MS: u64 = TTL_MS + STALE_MS;

pub(crate) const AT_TTL_BOUNDARY: u64 = STORED_AT_MS + TTL_MS;
pub(crate) const ONE_MS_PAST_TTL: u64 = AT_TTL_BOUNDARY + 1;
pub(crate) const AT_SWR_BOUNDARY: u64 = STORED_AT_MS + SWR_TOTAL_MS;
pub(crate) const ONE_MS_PAST_SWR: u64 = AT_SWR_BOUNDARY + 1;

pub(crate) fn ttl_resource() -> QueryResource<&'static str> {
    test_resource()
}

pub(crate) fn swr_resource() -> QueryResource<&'static str> {
    QueryResource::new(
        "swr-test",
        CachePolicy::StaleWhileRevalidate {
            ttl_ms: TTL_MS,
            stale_ms: STALE_MS,
        },
        RequestPolicy::LatestWins,
    )
}

pub(crate) fn nocache_test_resource() -> QueryResource<&'static str> {
    nocache_resource("nocache-test")
}

pub(crate) fn seed_data(
    resource: &mut QueryResource<&'static str>,
    data: &'static str,
    stored_at_ms: u64,
) {
    resource.apply_success(data, stored_at_ms);
}

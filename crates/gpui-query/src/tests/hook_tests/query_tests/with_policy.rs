use gpui::{AppContext as _, Entity, TestAppContext};

use crate::core::{CachePolicy, Fetched, QueryError, QueryResource, QueryStatus};
use crate::hook::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_use_query_with_policy_overrides_cache_policy(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_with_policy(
            QueryOptions::new("override").cache_policy(CachePolicy::NoCache),
            |_signal| async move {
                Ok::<_, QueryError>(Fetched::with_policy(
                    "data",
                    CachePolicy::Ttl { ttl_ms: 60_000 },
                ))
            },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.cache_policy(), CachePolicy::Ttl { ttl_ms: 60_000 });
        assert_eq!(resource.data(), Some(&"data"));
    });
}

#[gpui::test]
fn test_use_query_with_policy_none_keeps_caller_policy(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let caller_policy = CachePolicy::Ttl { ttl_ms: 5_000 };
    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_with_policy(
            QueryOptions::new("keep").cache_policy(caller_policy),
            |_signal| async move { Ok::<_, QueryError>(Fetched::new("data")) },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.cache_policy(), caller_policy);
        assert_eq!(resource.data(), Some(&"data"));
    });
}

#[gpui::test]
fn test_use_query_with_policy_stale_while_revalidate_override(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<u32, QueryError>>,
    }

    let server_policy = CachePolicy::StaleWhileRevalidate {
        ttl_ms: 30_000,
        stale_ms: 60_000,
    };
    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_with_policy(
            QueryOptions::new("swr").cache_policy(CachePolicy::NoCache),
            move |_signal| {
                let server_policy = server_policy;
                async move { Ok::<_, QueryError>(Fetched::with_policy(42_u32, server_policy)) }
            },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.cache_policy(), server_policy);
        assert_eq!(resource.data(), Some(&42));
    });
}

#[gpui::test]
fn test_fetch_query_with_policy_overrides_on_refetch(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query(
            QueryOptions::new("refetch-override").cache_policy(CachePolicy::NoCache),
            |_signal| async move { Ok::<_, QueryError>("first") },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.cache_policy(), CachePolicy::NoCache);
        assert_eq!(resource.data(), Some(&"first"));
    });

    harness.update(cx, |this, cx| {
        fetch_query_with_policy(
            &this.entity,
            || async move {
                Ok::<_, QueryError>(Fetched::with_policy(
                    "second",
                    CachePolicy::Ttl { ttl_ms: 60_000 },
                ))
            },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.cache_policy(), CachePolicy::Ttl { ttl_ms: 60_000 });
        assert_eq!(resource.data(), Some(&"second"));
    });
}

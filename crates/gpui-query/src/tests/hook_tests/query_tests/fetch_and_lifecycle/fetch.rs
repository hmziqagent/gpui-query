use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, BorrowAppContext as _, Entity, TestAppContext};

use crate::client::QueryClient;
use crate::core::{
    CachePolicy, QueryBeginResult, QueryError, QueryFetchMode, QueryKey, QueryResource,
    QuerySignal, QueryStatus, RequestId, RequestPolicy, RequestSequencer,
};
use crate::hook::*;
use crate::tests::test_support::*;

fn stale_prefetched_resource_with_inflight_revalidate(
    cx: &mut TestAppContext,
    key: &'static str,
    request_policy: RequestPolicy,
) -> (
    Entity<QueryResource<String, QueryError>>,
    RequestId,
    Option<QuerySignal>,
) {
    let policy = CachePolicy::StaleWhileRevalidate {
        ttl_ms: 1,
        stale_ms: 60_000,
    };
    let entity = cx.update(|cx| {
        let prepared = cx
            .update_global::<QueryClient, _>(|client, cx| {
                client.prepare_prefetch_query::<String, QueryError>(
                    QueryKey::from(key),
                    policy,
                    request_policy,
                    cx,
                )
            })
            .expect("prefetch starts on empty cache");
        let entity = prepared.entity.clone();
        prepared.complete_success("v1".to_string(), cx);
        entity.update(cx, |r, _| {
            r.apply_success("v1".to_string(), current_time_ms().saturating_sub(2_000));
        });
        entity
    });

    let (inflight_id, inflight_signal) = cx.update(|cx| {
        entity.update(cx, |r, _| {
            let mut seq = RequestSequencer::new();
            match r.begin_request(&mut seq, current_time_ms(), QueryFetchMode::Normal) {
                QueryBeginResult::Started { request_id, .. }
                | QueryBeginResult::StaleCacheHit { request_id, .. } => {
                    (request_id, r.signal().cloned())
                }
                other => panic!("expected in-flight revalidate start, got {other:?}"),
            }
        })
    });
    (entity, inflight_id, inflight_signal)
}

#[gpui::test]
fn test_use_query_manual_no_auto_fetch_then_manual_fetch(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<&'static str, QueryError, _>(
            QueryKey::from("manual-no-auto"),
            CachePolicy::Ttl { ttl_ms: 0 },
            RequestPolicy::LatestWins,
            cx,
        );
        assert_eq!(entity.read(cx).status(), QueryStatus::Idle);
        assert!(entity.read(cx).data().is_none());
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(
            harness.read(cx).entity.read(cx).status(),
            QueryStatus::Idle,
            "use_query_manual should never auto-fetch"
        );
    });

    harness.update(cx, |this, cx| {
        fetch_query(
            &this.entity,
            || async { Ok::<_, QueryError>("manual-result") },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.data(), Some(&"manual-result"));
    });
}

#[gpui::test]
fn test_use_query_manual_multiple_fetches(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let call_count = Arc::new(Mutex::new(0u32));
    let cc1 = call_count.clone();
    let cc2 = call_count.clone();

    struct H {
        entity: Entity<QueryResource<u32, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<u32, QueryError, _>(
            QueryKey::from("multi-manual"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        H { entity }
    });

    let cc_first = cc1.clone();
    harness.update(cx, |this, cx| {
        fetch_query(
            &this.entity,
            move || {
                let cc_first = cc_first.clone();
                async move {
                    *cc_first.lock().unwrap() += 1;
                    Ok::<_, QueryError>(1_u32)
                }
            },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(harness.read(cx).entity.read(cx).data(), Some(&1));
    });

    harness.update(cx, |this, cx| {
        fetch_query(
            &this.entity,
            move || {
                let cc2 = cc2.clone();
                async move {
                    *cc2.lock().unwrap() += 1;
                    Ok::<_, QueryError>(2_u32)
                }
            },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(harness.read(cx).entity.read(cx).data(), Some(&2));
    });
    assert_eq!(
        *call_count.lock().unwrap(),
        2,
        "both fetches should have executed"
    );
}

#[gpui::test]
fn test_fetch_query_on_idle_entity(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<String, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<String, QueryError, _>(
            QueryKey::from("fresh-key"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        assert_eq!(entity.read(cx).status(), QueryStatus::Idle);

        fetch_query(
            &entity,
            || async { Ok::<_, QueryError>("fresh-data".to_string()) },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.data(), Some(&"fresh-data".to_string()));
    });
}

#[gpui::test]
fn test_fetch_query_after_resource_reset(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query(
            QueryOptions::new("reset-test").cache_policy(CachePolicy::NoCache),
            |_signal| async move { Ok::<_, QueryError>("initial") },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(harness.read(cx).entity.read(cx).data(), Some(&"initial"));
    });
    let entity = cx.update(|cx| harness.read(cx).entity.clone());
    cx.update(|cx| {
        entity.update(cx, |r, _| {
            r.reset();
        });
    });

    cx.update(|cx| {
        assert_eq!(harness.read(cx).entity.read(cx).status(), QueryStatus::Idle);
    });

    harness.update(cx, |this, cx| {
        fetch_query(
            &this.entity,
            || async { Ok::<_, QueryError>("after-reset") },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.data(), Some(&"after-reset"));
    });
}

#[gpui::test]
fn test_fetch_query_concurrent_calls_latest_wins(cx: &mut TestAppContext) {
    setup_test(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let gate = Gate::new();
    let gate_clone = gate.clone();
    let executor = cx.background_executor.clone();

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<&'static str, QueryError, _>(
            QueryKey::from("concurrent-fetch"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        let executor = executor.clone();
        fetch_query(
            &entity,
            move || {
                let gate_clone = gate_clone.clone();
                let executor = executor.clone();
                async move {
                    gate_clone.wait(&executor).await;
                    Ok::<_, QueryError>("first")
                }
            },
            cx,
        );
        fetch_query(&entity, || async { Ok::<_, QueryError>("second") }, cx);
        H { entity }
    });

    gate.release();

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(
            resource.data(),
            Some(&"second"),
            "LatestWins: second fetch should be the winner"
        );
    });
}

#[gpui::test]
fn test_fetch_query_with_signal_completes(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<&'static str, QueryError, _>(
            QueryKey::from("signal-fetch"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        fetch_query_with_signal(
            &entity,
            |_signal| async { Ok::<_, QueryError>("signal-result") },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.data(), Some(&"signal-result"));
    });
}

#[gpui::test]
fn test_fetch_query_with_signal_failure(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<&'static str, QueryError, _>(
            QueryKey::from("signal-fail"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        fetch_query_with_signal(
            &entity,
            |_signal| async { Err::<&'static str, _>(QueryError::response("signal-error")) },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Failure);
        let err = resource.error().expect("should have error");
        assert!(err.to_string().contains("signal-error"));
    });
}

#[gpui::test]
fn fetch_query_while_loading_ignore_policy_does_not_spawn_duplicate_fetcher(
    cx: &mut TestAppContext,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<String, QueryError>>,
    }

    let count = Arc::new(AtomicUsize::new(0));
    let (entity, inflight_id, inflight_signal) = stale_prefetched_resource_with_inflight_revalidate(
        cx,
        "dup-ignore",
        RequestPolicy::IgnoreWhileLoading,
    );
    let harness = cx.new(|_| H { entity });

    let count_clone = count.clone();
    cx.update(|cx| {
        harness.update(cx, |h, cx| {
            fetch_query(
                &h.entity,
                move || {
                    let c = count_clone.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, QueryError>("v2".to_string())
                    }
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "fetch_query spawned a duplicate fetcher while a revalidate was \
             in flight under IgnoreWhileLoading"
        );
        assert!(
            !inflight_signal.unwrap().is_cancelled(),
            "the in-flight revalidate must keep its signal"
        );
        assert_eq!(resource.active_request_id(), Some(inflight_id));
        assert_eq!(resource.data(), Some(&"v1".to_string()));
    });
}

#[gpui::test]
fn fetch_query_while_loading_latest_wins_replaces_in_flight_request(cx: &mut TestAppContext) {
    use std::sync::atomic::{AtomicUsize, Ordering};

    setup_query_client(cx);

    struct H {
        entity: Entity<QueryResource<String, QueryError>>,
    }

    let count = Arc::new(AtomicUsize::new(0));
    let (entity, _inflight_id, inflight_signal) =
        stale_prefetched_resource_with_inflight_revalidate(
            cx,
            "dup-latest",
            RequestPolicy::LatestWins,
        );
    let harness = cx.new(|_| H { entity });

    let count_clone = count.clone();
    cx.update(|cx| {
        harness.update(cx, |h, cx| {
            fetch_query(
                &h.entity,
                move || {
                    let c = count_clone.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, QueryError>("v2".to_string())
                    }
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "LatestWins must replace the in-flight revalidate with a fresh fetch"
        );
        assert!(
            inflight_signal.unwrap().is_cancelled(),
            "LatestWins must cancel the replaced request's signal"
        );
        assert_eq!(resource.data(), Some(&"v2".to_string()));
    });
}

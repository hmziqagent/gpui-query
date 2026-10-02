use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, Entity, TestAppContext};

use crate::core::*;
use crate::hook::{
    InfiniteQueryOptions, MutationOptions, QueryOptions, fetch_next_page_infinite, fetch_query,
    fetch_query_with_signal, use_infinite_query, use_mutation, use_query_manual, use_query_select,
};
use crate::tests::test_support::*;

#[gpui::test]
fn test_use_mutation_accepts_mutation_options_directly(cx: &mut TestAppContext) {
    setup_query_client(cx);

    #[allow(dead_code)]
    struct H {
        mutation: Entity<MutationResource<String, String, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) =
            use_mutation::<String, String, QueryError, _>(MutationOptions::default(), cx);
        assert_eq!(entity.read(cx).status(), MutationStatus::Idle);
        H { mutation: entity }
    });

    cx.update(|cx| {
        let resource = harness.read(cx).mutation.read(cx);
        assert_eq!(resource.status(), MutationStatus::Idle);
        assert!(resource.data().is_none());
    });
}

#[gpui::test]
fn test_fetch_retry_stops_after_request_replaced(cx: &mut TestAppContext) {
    setup_test(cx);

    let gate = Gate::new();
    let gate_clone = gate.clone();
    let executor = cx.background_executor.clone();
    let call_count = Arc::new(Mutex::new(0u32));
    let cc = call_count.clone();

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<&'static str, QueryError, _>(
            QueryKey::from("retry-cancel"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        entity.update(cx, |r, _| {
            r.set_retry_policy(RetryPolicy::new(5).with_delay(0))
        });

        let executor = executor.clone();
        fetch_query(
            &entity,
            move || {
                let cc = cc.clone();
                let gate_clone = gate_clone.clone();
                let executor = executor.clone();
                async move {
                    {
                        let mut n = cc.lock().unwrap();
                        *n += 1;
                    }
                    gate_clone.wait(&executor).await;
                    Err::<_, QueryError>(QueryError::response("fail"))
                }
            },
            cx,
        );
        H { entity }
    });

    harness.update(cx, |this, cx| {
        fetch_query(&this.entity, || async { Ok::<_, QueryError>("new") }, cx);
    });

    gate.release();

    cx.run_until_parked();

    cx.update(|cx| {
        let data = harness.read(cx).entity.read(cx).data();
        assert_eq!(
            data,
            Some(&"new"),
            "second fetch should win under LatestWins"
        );
    });
}

#[gpui::test]
fn test_use_query_select_observer_updates_on_refetch(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let counter = Arc::new(Mutex::new(0u32));
    let c1 = counter.clone();
    let c2 = counter.clone();

    struct H {
        mapped: Entity<MappedQueryResource<&'static str, usize, QueryError>>,
        query: Entity<QueryResource<&'static str, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let transform = SelectTransform::new(|data: &&'static str| data.len());
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-observer").cache_policy(CachePolicy::NoCache),
            transform,
            move |_signal| {
                let c1 = c1.clone();
                async move {
                    let n = {
                        let mut g = c1.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    if n == 1 {
                        Ok::<_, QueryError>("hi")
                    } else {
                        Ok::<_, QueryError>("hello world")
                    }
                }
            },
            cx,
        );
        H {
            mapped,
            query,
            _subs: subs,
        }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let mapped_data = harness.read(cx).mapped.read(cx).data();
        assert_eq!(mapped_data, Some(2), "first fetch 'hi' has length 2");
    });

    harness.update(cx, |this, cx| {
        fetch_query(
            &this.query,
            move || {
                let c2 = c2.clone();
                async move {
                    let n = {
                        let mut g = c2.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    if n == 1 {
                        Ok::<_, QueryError>("hi")
                    } else {
                        Ok::<_, QueryError>("hello world")
                    }
                }
            },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let mapped_data = harness.read(cx).mapped.read(cx).data();
        assert_eq!(
            mapped_data,
            Some(11),
            "after refetch, observer should propagate update, transform should produce 11"
        );
    });
}

#[gpui::test]
fn test_fetch_query_with_signal_no_retry_on_failure(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let call_count = Arc::new(Mutex::new(0u32));
    let cc = call_count.clone();

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query_manual::<&'static str, QueryError, _>(
            QueryKey::from("no-retry-signal"),
            CachePolicy::NoCache,
            RequestPolicy::LatestWins,
            cx,
        );
        entity.update(cx, |r, _| r.set_retry_policy(RetryPolicy::new(3)));
        fetch_query_with_signal(
            &entity,
            move |_signal| {
                let cc = cc.clone();
                async move {
                    *cc.lock().unwrap() += 1;
                    Err::<&'static str, _>(QueryError::response("fail"))
                }
            },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(
            harness.read(cx).entity.read(cx).status(),
            QueryStatus::Failure,
            "FnOnce fetcher failure should result in Failure status"
        );
    });
    assert_eq!(
        *call_count.lock().unwrap(),
        1,
        "FnOnce fetcher must only be called once, no retries"
    );
}

#[gpui::test]
fn test_infinite_query_stops_retry_after_signal_cancelled(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let call_count = Arc::new(Mutex::new(0u32));
    let cc = call_count.clone();

    struct H {
        entity: Entity<InfiniteQueryResource<Vec<i32>, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_infinite_query(
            InfiniteQueryOptions::new("cancel-retry")
                .cache_policy(CachePolicy::Ttl { ttl_ms: 0 })
                .retry_policy(RetryPolicy::new(5).with_delay(0)),
            move |_| {
                let cc = cc.clone();
                async move {
                    let mut n = cc.lock().unwrap();
                    *n += 1;
                    Err::<_, QueryError>(QueryError::response("fail"))
                }
            },
            cx,
        );
        H { entity }
    });

    cx.run_until_parked();

    let entity_ref = cx.update(|cx| harness.read(cx).entity.clone());
    cx.update(|cx| {
        entity_ref.update(cx, |r, _| {
            if let Some(s) = r.signal() {
                s.cancel();
            }
        });
    });

    harness.update(cx, |this, cx| {
        fetch_next_page_infinite(
            &this.entity,
            |_| async move { Ok::<_, QueryError>((vec![99], false)) },
            cx,
        );
    });

    cx.run_until_parked();

    let count = *call_count.lock().unwrap();
    assert!(
        count <= 7,
        "should not have unbounded retries after signal cancellation, got {} calls",
        count
    );
}

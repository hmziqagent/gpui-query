use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, BorrowAppContext as _, Entity, TestAppContext};

use crate::client::QueryClient;
use crate::core::{
    CachePolicy, MappedQueryResource, QueryError, QueryResource, QueryStatus, RetryPolicy,
    SelectTransform,
};
use crate::hook::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_use_query_select_transform_applied(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        mapped: Entity<MappedQueryResource<Vec<String>, usize, QueryError>>,
        query: Entity<QueryResource<Vec<String>, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let transform = SelectTransform::new(|data: &Vec<String>| data.len());
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-test").cache_policy(CachePolicy::Ttl { ttl_ms: 0 }),
            transform,
            |_signal| async move { Ok::<_, QueryError>(vec!["a".to_string(), "b".to_string()]) },
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
        let h = harness.read(cx);
        let query_status = h.query.read(cx).status();
        assert_eq!(query_status, QueryStatus::Success);

        let mapped_data = h.mapped.read(cx).data();
        assert_eq!(
            mapped_data,
            Some(2),
            "transform should produce the length of the vec"
        );
    });
}

#[gpui::test]
fn test_use_query_select_transform_updated_on_refetch(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let counter = Arc::new(Mutex::new(0u32));
    let c1 = counter.clone();
    let c2 = counter.clone();

    struct H {
        mapped: Entity<MappedQueryResource<Vec<String>, usize, QueryError>>,
        query: Entity<QueryResource<Vec<String>, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let transform = SelectTransform::new(|data: &Vec<String>| data.len());
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-update").cache_policy(CachePolicy::NoCache),
            transform,
            move |_signal| {
                let c1 = c1.clone();
                async move {
                    let n = {
                        let mut g = c1.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    let items: Vec<String> = (0..n).map(|i| format!("item-{}", i)).collect();
                    Ok::<_, QueryError>(items)
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
        let h = harness.read(cx);
        let mapped_data = h.mapped.read(cx).data();
        assert_eq!(mapped_data, Some(1), "first fetch should have 1 item");
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
                    let items: Vec<String> = (0..n).map(|i| format!("item-{}", i)).collect();
                    Ok::<_, QueryError>(items)
                }
            },
            cx,
        );
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let h = harness.read(cx);
        let mapped_data = h.mapped.read(cx).data();
        assert_eq!(
            mapped_data,
            Some(2),
            "after refetch, transform should produce 2"
        );
    });
}

#[gpui::test]
fn test_use_query_select_memoization_consistency(cx: &mut TestAppContext) {
    setup_query_client(cx);

    #[allow(dead_code)]
    struct H {
        mapped: Entity<MappedQueryResource<&'static str, usize, QueryError>>,
        query: Entity<QueryResource<&'static str, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let transform = SelectTransform::new(|data: &&'static str| data.len());
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-memo").cache_policy(CachePolicy::Ttl { ttl_ms: 0 }),
            transform,
            |_signal| async move { Ok::<_, QueryError>("hello") },
            cx,
        );
        H {
            mapped,
            query,
            _subs: subs,
        }
    });

    cx.run_until_parked();

    let result1 = cx.update(|cx| harness.read(cx).mapped.read(cx).data());
    let result2 = cx.update(|cx| harness.read(cx).mapped.read(cx).data());

    assert_eq!(
        result1, result2,
        "repeated reads should produce the same result"
    );
    assert_eq!(result1, Some(5), "length of 'hello' is 5");
}

#[gpui::test]
fn test_use_query_select_handles_fetch_failure(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        mapped: Entity<MappedQueryResource<Vec<String>, usize, QueryError>>,
        query: Entity<QueryResource<Vec<String>, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let transform = SelectTransform::new(|data: &Vec<String>| data.len());
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-fail")
                .cache_policy(CachePolicy::Ttl { ttl_ms: 0 })
                .retry_policy(RetryPolicy::no_retries()),
            transform,
            |_signal| async move { Err::<_, QueryError>(QueryError::response("select-err")) },
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
        let h = harness.read(cx);
        let query_status = h.query.read(cx).status();
        assert_eq!(query_status, QueryStatus::Failure);

        let mapped_data = h.mapped.read(cx).data();
        assert_eq!(
            mapped_data, None,
            "mapped data should be None when query fails"
        );
    });
}

#[gpui::test]
fn test_use_query_select_multiple_transforms_same_query(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let fetch_count = Arc::new(Mutex::new(0u32));
    let fc1 = fetch_count.clone();
    let fc2 = fetch_count.clone();

    #[allow(dead_code)]
    struct H {
        mapped_len: Entity<MappedQueryResource<Vec<String>, usize, QueryError>>,
        mapped_first: Entity<MappedQueryResource<Vec<String>, Option<String>, QueryError>>,
        query: Entity<QueryResource<Vec<String>, QueryError>>,
        _subs_len: (gpui::Subscription, gpui::Subscription),
        _subs_first: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let (mapped_len, query, subs_len) = use_query_select(
            QueryOptions::new("multi-select").cache_policy(CachePolicy::Ttl { ttl_ms: 60_000 }),
            SelectTransform::new(|data: &Vec<String>| data.len()),
            move |_signal| {
                let fc1 = fc1.clone();
                async move {
                    let n = {
                        let mut g = fc1.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    let items: Vec<String> = (0..n + 2).map(|i| format!("item-{}", i)).collect();
                    Ok::<_, QueryError>(items)
                }
            },
            cx,
        );

        let (mapped_first, query2, subs_first) = use_query_select(
            QueryOptions::new("multi-select").cache_policy(CachePolicy::Ttl { ttl_ms: 60_000 }),
            SelectTransform::new(|data: &Vec<String>| data.first().cloned()),
            move |_signal| {
                let fc2 = fc2.clone();
                async move {
                    let n = {
                        let mut g = fc2.lock().unwrap();
                        *g += 1;
                        *g
                    };
                    let items: Vec<String> = (0..n + 2).map(|i| format!("item-{}", i)).collect();
                    Ok::<_, QueryError>(items)
                }
            },
            cx,
        );

        assert_eq!(
            query.entity_id(),
            query2.entity_id(),
            "same key should return same query entity"
        );

        H {
            mapped_len,
            mapped_first,
            query,
            _subs_len: subs_len,
            _subs_first: subs_first,
        }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let h = harness.read(cx);
        let len = h.mapped_len.read(cx).data();
        let first = h.mapped_first.read(cx).data();
        assert_eq!(
            len,
            Some(3),
            "length transform should produce 3 (from first fetch only)"
        );
        assert_eq!(
            first,
            Some(Some("item-0".to_string())),
            "first transform should produce Some('item-0') (from first fetch only)"
        );
    });
    assert_eq!(
        *fetch_count.lock().unwrap(),
        1,
        "only one fetch should have occurred — second select must be a cache hit"
    );
}

#[gpui::test]
fn use_query_select_propagates_optimistic_set_query_data(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        mapped: Entity<MappedQueryResource<String, usize, QueryError>>,
        _query: Entity<QueryResource<String, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let harness = cx.new(|cx| {
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-optimistic").cache_policy(CachePolicy::Ttl { ttl_ms: 0 }),
            SelectTransform::new(|data: &String| data.len()),
            |_signal| async move { Ok::<_, QueryError>("first".to_string()) },
            cx,
        );
        H {
            mapped,
            _query: query,
            _subs: subs,
        }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(harness.read(cx).mapped.read(cx).data(), Some(5));
    });

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>(
                "select-optimistic",
                "second-value".to_string(),
                cx,
            );
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(
            harness.read(cx).mapped.read(cx).data(),
            Some(12),
            "same-status optimistic write must propagate through the select projection"
        );
    });
}

#[gpui::test]
fn use_query_select_propagates_equal_value_write_via_data_epoch(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        mapped: Entity<MappedQueryResource<String, usize, QueryError>>,
        _query: Entity<QueryResource<String, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
        sub2: Option<gpui::Subscription>,
        counter: Arc<AtomicUsize>,
    }

    let harness = cx.new(|cx| {
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-equal-write"),
            SelectTransform::new(|data: &String| data.len()),
            |_signal| async move { Ok::<_, QueryError>("same".to_string()) },
            cx,
        );
        H {
            mapped,
            _query: query,
            _subs: subs,
            sub2: None,
            counter: Arc::new(AtomicUsize::new(0)),
        }
    });

    cx.run_until_parked();

    harness.update(cx, |h, cx| {
        let counter = h.counter.clone();
        h.sub2 = Some(cx.observe(&h.mapped, move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
    });

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>(
                "select-equal-write",
                "same".to_string(),
                cx,
            );
        });
    });
    cx.run_until_parked();

    assert_eq!(
        harness.read_with(cx, |h, _| h.counter.load(Ordering::SeqCst)),
        1,
        "an equal-value data write still moves the data epoch and must reach the mapped entity"
    );
}

struct CloneCounting {
    value: u32,
    clones: Arc<AtomicUsize>,
}

impl Clone for CloneCounting {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            value: self.value,
            clones: Arc::clone(&self.clones),
        }
    }
}

impl PartialEq for CloneCounting {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

#[gpui::test]
fn use_query_select_skips_clone_on_notify_without_data_write(cx: &mut TestAppContext) {
    setup_query_client(cx);

    struct H {
        mapped: Entity<MappedQueryResource<CloneCounting, u32, QueryError>>,
        query: Entity<QueryResource<CloneCounting, QueryError>>,
        _subs: (gpui::Subscription, gpui::Subscription),
    }

    let clones = Arc::new(AtomicUsize::new(0));
    let fetched = CloneCounting {
        value: 7,
        clones: clones.clone(),
    };

    let harness = cx.new(|cx| {
        let (mapped, query, subs) = use_query_select(
            QueryOptions::new("select-no-clone"),
            SelectTransform::new(|data: &CloneCounting| data.value),
            move |_signal| {
                let fetched = fetched.clone();
                async move { Ok::<_, QueryError>(fetched) }
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

    let baseline = clones.load(Ordering::SeqCst);
    assert!(
        baseline > 0,
        "the fetch result was cloned into the projection"
    );

    let query_entity = cx.update(|cx| harness.read(cx).query.clone());
    for _ in 0..5 {
        cx.update(|cx| {
            query_entity.update(cx, |_, cx| cx.notify());
        });
    }

    assert_eq!(
        clones.load(Ordering::SeqCst),
        baseline,
        "notifies that carry no data write must not re-clone T into the projection"
    );
    cx.update(|cx| {
        assert_eq!(harness.read(cx).mapped.read(cx).data(), Some(7));
    });
}

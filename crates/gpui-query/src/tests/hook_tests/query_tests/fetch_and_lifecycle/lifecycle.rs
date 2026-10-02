use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, Entity, TestAppContext};

use crate::core::{CachePolicy, QueryError, QueryResource, QueryStatus, RequestPolicy};
use crate::hook::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_use_query_ignore_while_loading_policy(cx: &mut TestAppContext) {
    setup_test(cx);

    let fetch_count = Arc::new(Mutex::new(0u32));
    let fc1 = fetch_count.clone();
    let fc2 = fetch_count.clone();

    let gate = Gate::new();
    let gate_clone = gate.clone();
    let executor = cx.background_executor.clone();

    struct H {
        entity: Entity<QueryResource<&'static str, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) = use_query(
            QueryOptions::new("ignore-loading")
                .cache_policy(CachePolicy::NoCache)
                .request_policy(RequestPolicy::IgnoreWhileLoading),
            move |_signal| {
                let fc1 = fc1.clone();
                let gate_clone = gate_clone.clone();
                let executor = executor.clone();
                async move {
                    *fc1.lock().unwrap() += 1;
                    gate_clone.wait(&executor).await;
                    Ok::<_, QueryError>("first-fetch")
                }
            },
            cx,
        );
        H { entity }
    });

    harness.update(cx, |this, cx| {
        fetch_query(
            &this.entity,
            move || {
                let fc2 = fc2.clone();
                async move {
                    *fc2.lock().unwrap() += 1;
                    Ok::<_, QueryError>("ignored-fetch")
                }
            },
            cx,
        );
    });

    gate.release();

    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(resource.status(), QueryStatus::Success);
        assert_eq!(resource.data(), Some(&"first-fetch"));
    });
    assert_eq!(
        *fetch_count.lock().unwrap(),
        1,
        "IgnoreWhileLoading should have rejected the second fetch"
    );
}

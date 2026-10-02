use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, Entity, TestAppContext};

use crate::core::{MutationResource, MutationStatus, QueryError};
use crate::hook::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_mutate_callbacks_all_fire_on_success(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let success_val = Arc::new(Mutex::new(String::new()));
    let error_val = Arc::new(Mutex::new(String::new()));
    let settled_data = Arc::new(Mutex::new(None::<String>));
    let settled_err = Arc::new(Mutex::new(None::<String>));

    let sv = success_val.clone();
    let ev = error_val.clone();
    let sd = settled_data.clone();
    let se = settled_err.clone();

    #[allow(dead_code)]
    struct H {
        mutation: Entity<MutationResource<String, String, QueryError>>,
    }

    let _harness = cx.new(|cx| {
        let (entity, _sub) = use_mutation::<String, String, QueryError, _>((), cx);
        mutate_with_callbacks(
            &entity,
            "all-cb".to_string(),
            |v| async move { Ok::<_, QueryError>(format!("yes-{}", v)) },
            MutationCallbacks::<String, QueryError>::new()
                .on_success(move |data: &String| {
                    *sv.lock().unwrap() = data.clone();
                })
                .on_error(move |err: &QueryError| {
                    *ev.lock().unwrap() = err.to_string();
                })
                .on_settled(
                    move |opt_data: Option<&String>, opt_err: Option<&QueryError>| {
                        *sd.lock().unwrap() = opt_data.map(|d| d.to_string());
                        *se.lock().unwrap() = opt_err.map(|e| e.to_string());
                    },
                ),
            cx,
        );
        H { mutation: entity }
    });

    cx.run_until_parked();

    assert_eq!(
        *success_val.lock().unwrap(),
        "yes-all-cb",
        "on_success should fire with data"
    );
    assert!(
        error_val.lock().unwrap().is_empty(),
        "on_error should NOT fire on success"
    );
    assert_eq!(
        *settled_data.lock().unwrap(),
        Some("yes-all-cb".to_string()),
        "on_settled should receive data on success"
    );
    assert!(
        settled_err.lock().unwrap().is_none(),
        "on_settled should NOT receive error on success"
    );
}

#[gpui::test]
fn test_mutate_callbacks_all_fire_on_failure(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let success_fired = Arc::new(Mutex::new(false));
    let error_msg = Arc::new(Mutex::new(String::new()));
    let settled_data = Arc::new(Mutex::new(None::<String>));
    let settled_err = Arc::new(Mutex::new(None::<String>));

    let sf = success_fired.clone();
    let em = error_msg.clone();
    let sd = settled_data.clone();
    let se = settled_err.clone();

    #[allow(dead_code)]
    struct H {
        mutation: Entity<MutationResource<String, String, QueryError>>,
    }

    let _harness = cx.new(|cx| {
        let (entity, _sub) =
            use_mutation::<String, String, QueryError, _>(no_retry_mutation_options(), cx);
        mutate_with_callbacks(
            &entity,
            "fail-cb".to_string(),
            |_| async { Err::<String, _>(QueryError::response("total-failure")) },
            MutationCallbacks::<String, QueryError>::new()
                .on_success(move |_data: &String| {
                    *sf.lock().unwrap() = true;
                })
                .on_error(move |err: &QueryError| {
                    *em.lock().unwrap() = err.to_string();
                })
                .on_settled(
                    move |opt_data: Option<&String>, opt_err: Option<&QueryError>| {
                        *sd.lock().unwrap() = opt_data.map(|d| d.to_string());
                        *se.lock().unwrap() = opt_err.map(|e| e.to_string());
                    },
                ),
            cx,
        );
        H { mutation: entity }
    });

    cx.run_until_parked();

    assert!(
        !*success_fired.lock().unwrap(),
        "on_success should NOT fire on failure"
    );
    assert!(
        error_msg.lock().unwrap().contains("total-failure"),
        "on_error should fire with error message"
    );
    assert!(
        settled_data.lock().unwrap().is_none(),
        "on_settled should NOT receive data on failure"
    );
    assert!(
        settled_err.lock().unwrap().is_some(),
        "on_settled should receive error on failure"
    );
}

#[gpui::test]
fn test_mutate_callbacks_settled_always_fires_on_success(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let settled_called = Arc::new(Mutex::new(false));
    let sc = settled_called.clone();

    #[allow(dead_code)]
    struct H {
        mutation: Entity<MutationResource<String, String, QueryError>>,
    }

    let _harness = cx.new(|cx| {
        let (entity, _sub) = use_mutation::<String, String, QueryError, _>((), cx);
        mutate_with_callbacks(
            &entity,
            "settled-only".to_string(),
            |v| async move { Ok::<_, QueryError>(format!("s-{}", v)) },
            MutationCallbacks::<String, QueryError>::new().on_settled(move |opt_data, opt_err| {
                assert!(opt_data.is_some(), "settled should have data");
                assert!(opt_err.is_none(), "settled should not have error");
                *sc.lock().unwrap() = true;
            }),
            cx,
        );
        H { mutation: entity }
    });

    cx.run_until_parked();

    assert!(
        *settled_called.lock().unwrap(),
        "on_settled must fire on success even without on_success callback"
    );
}

#[gpui::test]
fn test_mutate_callbacks_settled_always_fires_on_failure(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let settled_called = Arc::new(Mutex::new(false));
    let sc = settled_called.clone();

    #[allow(dead_code)]
    struct H {
        mutation: Entity<MutationResource<String, String, QueryError>>,
    }

    let _harness = cx.new(|cx| {
        let (entity, _sub) =
            use_mutation::<String, String, QueryError, _>(no_retry_mutation_options(), cx);
        mutate_with_callbacks(
            &entity,
            "settled-fail".to_string(),
            |_| async { Err::<String, _>(QueryError::response("fail")) },
            MutationCallbacks::<String, QueryError>::new().on_settled(move |opt_data, opt_err| {
                assert!(
                    opt_data.is_none(),
                    "settled should not have data on failure"
                );
                assert!(opt_err.is_some(), "settled should have error on failure");
                *sc.lock().unwrap() = true;
            }),
            cx,
        );
        H { mutation: entity }
    });

    cx.run_until_parked();

    assert!(
        *settled_called.lock().unwrap(),
        "on_settled must fire on failure even without on_error callback"
    );
}

#[gpui::test]
fn test_mutate_late_ok_after_cancel_keeps_failure(cx: &mut TestAppContext) {
    setup_query_client(cx);

    let success_fired = Arc::new(Mutex::new(false));
    let error_msg = Arc::new(Mutex::new(String::new()));
    let settled_data = Arc::new(Mutex::new(None::<String>));
    let settled_err = Arc::new(Mutex::new(None::<String>));

    let sf = success_fired.clone();
    let em = error_msg.clone();
    let sd = settled_data.clone();
    let se = settled_err.clone();

    let gate = Gate::new();
    let gate_for_mutator = gate.clone();
    let executor = cx.background_executor.clone();

    #[allow(dead_code)]
    struct H {
        mutation: Entity<MutationResource<String, String, QueryError>>,
    }

    let harness = cx.new(|cx| {
        let (entity, _sub) =
            use_mutation::<String, String, QueryError, _>(no_retry_mutation_options(), cx);
        let gate_for_mutator = gate_for_mutator.clone();
        let executor = executor.clone();
        mutate_with_callbacks(
            &entity,
            "cancel-late-ok".to_string(),
            move |_| {
                let gate_for_mutator = gate_for_mutator.clone();
                let executor = executor.clone();
                async move {
                    gate_for_mutator.wait(&executor).await;
                    Ok::<_, QueryError>("late-success".to_string())
                }
            },
            MutationCallbacks::<String, QueryError>::new()
                .on_success(move |_data: &String| {
                    *sf.lock().unwrap() = true;
                })
                .on_error(move |err: &QueryError| {
                    *em.lock().unwrap() = err.to_string();
                })
                .on_settled(
                    move |opt_data: Option<&String>, opt_err: Option<&QueryError>| {
                        *sd.lock().unwrap() = opt_data.map(|d| d.to_string());
                        *se.lock().unwrap() = opt_err.map(|e| e.to_string());
                    },
                ),
            cx,
        );
        H { mutation: entity }
    });

    cx.update(|cx| {
        assert!(
            harness.read(cx).mutation.read(cx).is_loading(),
            "precondition: mutation Loading while parked on the gate"
        );
    });

    harness.update(cx, |h, cx| {
        h.mutation.update(cx, |m, _| {
            m.cancel(QueryError::cancelled("user aborted"));
        });
    });

    gate.release();
    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).mutation.read(cx);
        assert_eq!(
            resource.status(),
            MutationStatus::Failure,
            "the cancelled mutation's terminal Failure must not be overwritten \
             by the late Ok"
        );
        assert!(
            resource.data().is_none(),
            "no success data may appear after cancel"
        );
        assert!(
            error_msg.lock().unwrap().contains("user aborted"),
            "on_error fires with the cancel error"
        );
        assert!(
            !*success_fired.lock().unwrap(),
            "on_success must not fire for a cancelled mutation"
        );
        assert!(
            settled_data.lock().unwrap().is_none(),
            "on_settled must not receive the late success data"
        );
        assert!(
            settled_err.lock().unwrap().is_some(),
            "on_settled receives the cancel error"
        );
    });
}

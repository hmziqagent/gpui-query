//! Cached data projected into a derived shape, re-running only when the data
//! changes; the hook returns a `MappedQueryResource<T, U, E>`.

use std::cell::Cell;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, Subscription};

use crate::core::{MappedQueryResource, QueryResource, SelectTransform};

use super::{QueryOptions, use_query};

/// (mapped entity, source query entity, both subscriptions).
pub type QuerySelectResult<T, U, E> = (
    Entity<MappedQueryResource<T, U, E>>,
    Entity<QueryResource<T, E>>,
    (Subscription, Subscription),
);

/// The transform runs lazily on every `mapped.data()` call (no output cache), so reuse the result if it is expensive:
///
/// ```no_run
/// use gpui_query::core::{MappedQueryResource, SelectTransform};
/// # fn _doc(mapped: &gpui::Entity<MappedQueryResource<Vec<String>, usize, ()>>, cx: &gpui::App) {
/// let count = mapped.read(cx).data(); // transform runs once; reuse `count`
/// # }
/// ```
///
/// # Example
///
/// ```no_run
/// use gpui_query::hook::{use_query_select, QueryOptions};
/// use gpui_query::core::SelectTransform;
/// # #[derive(Clone, PartialEq)]
/// # struct User;
/// # #[derive(Clone, Debug)]
/// # struct MyError;
///
/// struct UserCountView {
///     mapped: gpui::Entity<gpui_query::core::MappedQueryResource<Vec<User>, usize, MyError>>,
///     _subs: (gpui::Subscription, gpui::Subscription),
/// }
///
/// impl UserCountView {
///     fn new(cx: &mut gpui::Context<Self>) -> Self {
///         let count_transform = SelectTransform::new(|users: &Vec<User>| users.len());
///         let (mapped, query_entity, subs) = use_query_select(
///             QueryOptions::new("users"),
///             count_transform,
///             |signal| async move {
///                 Ok(vec![])
///             },
///             cx,
///         );
///         Self { mapped, _subs: subs }
///     }
/// }
/// ```
pub fn use_query_select<T, U, E, C, F, Fut>(
    options: impl Into<QueryOptions>,
    transform: SelectTransform<T, U>,
    fetcher: F,
    cx: &mut Context<C>,
) -> QuerySelectResult<T, U, E>
where
    T: Clone + PartialEq + Send + Sync + 'static,
    U: 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(crate::core::QuerySignal) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let (query_entity, query_subscription) = use_query(options, fetcher, cx);

    let (initial_data, initial_epoch) = query_entity.read_with(cx, |r, _| {
        (r.data().map(|d| Arc::new(d.clone())), r.data_epoch())
    });
    let mapped = MappedQueryResource::new(initial_data, transform);
    let mapped_entity = cx.new(|_| mapped);

    let mapped_weak = mapped_entity.downgrade();
    let last_epoch = Cell::new(initial_epoch);
    let mapped_subscription = cx.observe(&query_entity, move |_, entity, cx| {
        let Some(mapped) = mapped_weak.upgrade() else {
            return;
        };

        // The data epoch is the change verdict: no deep-compare of `T` here,
        // and `T` is cloned only when a write actually moved the epoch.
        let update: Option<Option<Arc<T>>> = entity.read_with(cx, |r, _| {
            let epoch = r.data_epoch();
            (epoch != last_epoch.get()).then(|| {
                last_epoch.set(epoch);
                r.data().map(|d| Arc::new(d.clone()))
            })
        });

        if let Some(fresh) = update {
            mapped.update(cx, |m, cx| {
                m.update_source(fresh);
                cx.notify();
            });
        }
    });

    (
        mapped_entity,
        query_entity,
        (query_subscription, mapped_subscription),
    )
}

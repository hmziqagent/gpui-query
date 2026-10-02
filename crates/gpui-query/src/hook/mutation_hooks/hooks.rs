use std::sync::Arc;

use gpui::{AppContext as _, BorrowAppContext as _, Context, Entity, Subscription};

use crate::client::{MutationObserver, QueryClient};
use crate::core::MutationResource;

use super::super::MutationOptions;
use super::super::options::MutationCallbacks;
use super::internals::run_mutation_loop;

/// The observer dedupes on `MutationStatus` (retry ticks stay in Loading, no re-render); the entity registers with [`QueryClient`] so `use_mutation_state` finds it and GC respects `gc_time_ms`.
///
/// # Example
///
/// ```no_run
/// use gpui::{Entity, Subscription, Context};
/// use gpui_query::hook::{use_mutation, mutate};
/// use gpui_query::MutationResource;
/// # #[derive(Clone)]
/// # struct NewUser { name: String }
/// # #[derive(Clone)]
/// # struct User;
/// # #[derive(Clone, Debug)]
/// # struct MyError;
///
/// struct MyView {
///     create_user: Entity<MutationResource<NewUser, User, MyError>>,
///     _mutation_sub: Subscription,
/// }
///
/// impl MyView {
///     fn new(cx: &mut Context<Self>) -> Self {
///         let (entity, sub) = use_mutation((), cx);
///         Self { create_user: entity, _mutation_sub: sub }
///     }
///
///     fn handle_submit(&mut self, name: String, cx: &mut Context<Self>) {
///         mutate(&self.create_user, NewUser { name }, |vars| async move {
///             Ok(User)
///         }, cx);
///     }
/// }
/// ```
pub fn use_mutation<V, T, E, C>(
    options: impl Into<MutationOptions>,
    cx: &mut Context<C>,
) -> (Entity<MutationResource<V, T, E>>, Subscription)
where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: 'static,
{
    let opts = options.into();
    let entity = cx.new(|_| MutationResource::new(opts.retry_policy));

    let observer = MutationObserver::new(&entity);
    let Some(subscription) = observer.observe(cx) else {
        debug_assert!(
            false,
            "MutationObserver::observe failed: entity was just created and cannot be dropped"
        );
        return (entity, Subscription::new(|| {}));
    };

    if cx.has_global::<QueryClient>() {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_mutation(&entity, cx);
        });
    }

    (entity, subscription)
}

/// All registered mutations for the `(V, T, E)` triple; empty when none exist or no [`QueryClient`] is set.
///
/// # Example
///
/// ```no_run
/// use gpui_query::hook::use_mutation_state;
/// # #[derive(Clone)]
/// # struct NewUser;
/// # #[derive(Clone)]
/// # struct User;
/// # #[derive(Clone, Debug)]
/// # struct QueryError;
/// # fn _doc<C: 'static>(cx: &mut gpui::Context<C>) {
///
/// let mutations = use_mutation_state::<NewUser, User, QueryError, _>(cx);
/// # }
/// ```
pub fn use_mutation_state<V, T, E, C>(cx: &mut Context<C>) -> Vec<Entity<MutationResource<V, T, E>>>
where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
    C: 'static,
{
    if cx.has_global::<QueryClient>() {
        cx.read_global::<QueryClient, _>(|client, _| client.all_mutations::<V, T, E>())
    } else {
        Vec::new()
    }
}

/// Variables are wrapped in an `Arc<V>` (one `V::clone` per attempt); a call while Loading is a no-op, and a replacement call or entity drop aborts the prior task.
///
/// # Example
///
/// ```no_run
/// use gpui_query::hook::mutate;
/// # #[derive(Clone)]
/// # struct Vars;
/// # #[derive(Clone)]
/// # struct Data;
/// # #[derive(Clone, Debug)]
/// # struct Err;
/// # fn _doc(entity: &gpui::Entity<gpui_query::MutationResource<Vars, Data, Err>>, cx: &mut gpui::Context<()>) {
///
/// mutate(entity, Vars, |v| async move { Ok(Data) }, cx);
/// # }
/// ```
pub fn mutate<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: V,
    mutator: F,
    cx: &mut Context<C>,
) where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(V) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    begin_and_spawn(
        entity,
        Arc::new(variables),
        move |v: &V| mutator(v.clone()),
        cx,
        None,
    );
}

/// Callbacks fire on the terminal outcome only, outside any entity borrow (safe
/// to call `entity.update()`); `on_error`/`on_settled` still fire if the entity
/// drops mid-mutation.
pub fn mutate_with_callbacks<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: V,
    mutator: F,
    callbacks: MutationCallbacks<T, E>,
    cx: &mut Context<C>,
) where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(V) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    begin_and_spawn(
        entity,
        Arc::new(variables),
        move |v: &V| mutator(v.clone()),
        cx,
        Some(callbacks),
    );
}

/// The mutator receives `&V` borrowed from the stored `Arc<V>`: no `V::clone`
/// per attempt (`V: Clone` is still required; `begin` stores an owned copy).
pub fn mutate_by_ref<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: V,
    mutator: F,
    cx: &mut Context<C>,
) where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(&V) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    begin_and_spawn(entity, Arc::new(variables), mutator, cx, None);
}

/// [`mutate_by_ref`] taking `Arc<V>` directly, so callers share one variables buffer.
pub fn mutate_arc<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: Arc<V>,
    mutator: F,
    cx: &mut Context<C>,
) where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(&V) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    begin_and_spawn(entity, variables, mutator, cx, None);
}

/// The `is_loading` guard and `begin` run in one `entity.update`, so racing
/// callers cannot both begin; `set_current_task` makes replacement or drop abort.
fn begin_and_spawn<V, T, E, C, F, Fut>(
    entity: &Entity<MutationResource<V, T, E>>,
    variables: Arc<V>,
    mutator: F,
    cx: &mut Context<C>,
    callbacks: Option<MutationCallbacks<T, E>>,
) where
    V: Clone + Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    C: 'static,
    F: Fn(&V) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let began = entity.update(cx, |resource, cx| {
        if resource.is_loading() {
            return false;
        }
        resource.begin((*variables).clone());
        cx.notify();
        true
    });
    if !began {
        return;
    }

    let retry_policy = entity.read_with(cx, |r, _| r.retry_policy().clone());
    let weak = entity.downgrade();

    let task: gpui::Task<()> = cx.spawn(async move |_this, cx| {
        run_mutation_loop(&weak, variables, mutator, &retry_policy, callbacks, cx).await;
    });
    entity.update(cx, |r, _| {
        r.set_current_task(task);
    });
}

//! The `(T, E)`-typed query bucket: a facade over [`ResourceBucket`].

use gpui::{App, Entity};

use crate::core::{CachePolicy, QueryKey, QueryResource, RequestPolicy, RequestSequencer};

use super::shared::ResourceBucket;

pub struct QueryBucket<T, E> {
    pub(crate) inner: ResourceBucket<QueryResource<T, E>>,
}

impl<T: Clone + Send + Sync + 'static, E: Clone + Send + Sync + 'static> QueryBucket<T, E> {
    pub(crate) fn new() -> Self {
        Self {
            inner: ResourceBucket::new(),
        }
    }

    pub(crate) fn get_or_create(
        &mut self,
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> Entity<QueryResource<T, E>> {
        self.inner
            .get_or_create(key, cache_policy, request_policy, cx)
    }

    pub(crate) fn get_or_create_with_request_id(
        &mut self,
        key: QueryKey,
        cache_policy: CachePolicy,
        request_policy: RequestPolicy,
        cx: &mut App,
    ) -> (Entity<QueryResource<T, E>>, crate::core::RequestId) {
        self.inner
            .get_or_create_with_request_id(key, cache_policy, request_policy, cx)
    }

    pub(crate) fn get(&self, key: &QueryKey) -> Option<Entity<QueryResource<T, E>>> {
        self.inner.get(key)
    }

    pub(crate) fn sequencer_mut(&mut self, key: &QueryKey) -> Option<&mut RequestSequencer> {
        self.inner.sequencer_mut(key)
    }

    pub(crate) fn all_entities(&self) -> Vec<Entity<QueryResource<T, E>>> {
        self.inner.all_entities()
    }
}

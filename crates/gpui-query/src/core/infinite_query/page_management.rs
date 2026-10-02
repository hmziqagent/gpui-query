use std::sync::Arc;

use super::{FetchDirection, InfiniteQueryResource};

impl<T, E> InfiniteQueryResource<T, E> {
    pub fn set_has_next_page(&mut self, has_next: bool) {
        self.has_next_page = has_next;
    }

    pub fn set_has_previous_page(&mut self, has_prev: bool) {
        self.has_previous_page = has_prev;
    }

    /// Only changes what `reset()` restores the flags to; current flags stay.
    pub fn set_direction(&mut self, direction: FetchDirection) {
        self.direction = direction;
    }

    /// `Some(0)` is treated as unbounded (`None`) so a zero cannot drain all
    /// pages. Returns evicted pages (if any) as `Arc` handles.
    pub fn set_max_pages(&mut self, max: Option<usize>) -> Vec<Arc<T>> {
        self.max_pages = match max {
            Some(0) => None,
            other => other,
        };
        let evicted = self.enforce_max_pages_remove_front();
        if !evicted.is_empty() {
            self.data_epoch = self.data_epoch.saturating_add(1);
        }
        evicted
    }

    /// Returns evicted pages (if any) as `Arc` handles.
    pub fn append_page(&mut self, page: T) -> Vec<Arc<T>> {
        self.pages.push_back(Arc::new(page));
        self.data_epoch = self.data_epoch.saturating_add(1);
        self.enforce_max_pages_remove_front()
    }

    /// Returns evicted pages (if any) as `Arc` handles.
    pub fn prepend_page(&mut self, page: T) -> Vec<Arc<T>> {
        self.pages.push_front(Arc::new(page));
        self.data_epoch = self.data_epoch.saturating_add(1);
        self.enforce_max_pages_remove_back()
    }

    /// At least 1 page is always retained.
    pub(super) fn enforce_max_pages_remove_front(&mut self) -> Vec<Arc<T>> {
        if let Some(max) = self.max_pages
            && max > 0
            && self.pages.len() > max
        {
            return self.pages.drain(..self.pages.len() - max).collect();
        }
        Vec::new()
    }

    /// Survivors are the first `max` pages, so the drain starts at `max`, not
    /// `len - max`. Drained in reverse so the returned vector preserves
    /// most-recently-prepended-first eviction order without a second pass.
    pub(super) fn enforce_max_pages_remove_back(&mut self) -> Vec<Arc<T>> {
        if let Some(max) = self.max_pages
            && max > 0
            && self.pages.len() > max
        {
            return self.pages.drain(max..).rev().collect();
        }
        Vec::new()
    }
}

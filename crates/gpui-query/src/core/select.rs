//! Select/transform support: [`SelectTransform`] projects cached `T` into a
//! derived `U` via [`MappedQueryResource`] without duplicating the cache entry.

use std::sync::Arc;

/// Stored as `Arc<dyn Fn(&T) -> U + Send + Sync>` so it is `Clone`; combine
/// with [`MappedQueryResource`] to derive views without a second data copy.
///
/// # Example
///
/// ```
/// use gpui_query::core::SelectTransform;
///
/// let uppercase = SelectTransform::new(|name: &String| name.to_uppercase());
/// assert_eq!(uppercase.apply(&"hello".to_string()), "HELLO");
/// ```
pub struct SelectTransform<T, U> {
    transform: Arc<dyn Fn(&T) -> U + Send + Sync>,
    _marker: std::marker::PhantomData<(T, U)>,
}

impl<T, U> Clone for SelectTransform<T, U> {
    fn clone(&self) -> Self {
        Self {
            transform: Arc::clone(&self.transform),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T, U> std::fmt::Debug for SelectTransform<T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectTransform").finish()
    }
}

impl<T, U> PartialEq for SelectTransform<T, U> {
    fn eq(&self, other: &Self) -> bool {
        // Closures have no PartialEq; compare by pointer identity like QuerySignal.
        Arc::ptr_eq(&self.transform, &other.transform)
    }
}

impl<T, U> Eq for SelectTransform<T, U> {}

impl<T, U> SelectTransform<T, U> {
    pub fn new(transform: impl Fn(&T) -> U + Send + Sync + 'static) -> Self {
        Self {
            transform: Arc::new(transform),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn apply(&self, data: &T) -> U {
        (self.transform)(data)
    }
}

/// Several consumers can derive different views from one shared cache entry;
/// the source is held as `Option<Arc<T>>` so cloning is a refcount bump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappedQueryResource<T, U, E> {
    source_data: Option<Arc<T>>,
    transform: SelectTransform<T, U>,
    _error_marker: std::marker::PhantomData<E>,
}

impl<T, U, E> MappedQueryResource<T, U, E> {
    pub fn new(source_data: Option<Arc<T>>, transform: SelectTransform<T, U>) -> Self {
        Self {
            source_data,
            transform,
            _error_marker: std::marker::PhantomData,
        }
    }

    /// Re-applies the transform on every call; cache the result in a local
    /// if you need it repeatedly within one render pass.
    ///
    /// ```
    /// use gpui_query::core::{MappedQueryResource, SelectTransform};
    ///
    /// let transform = SelectTransform::new(|v: &Vec<i32>| v.len());
    /// let mapped = MappedQueryResource::<_, usize, ()>::new(Some(std::sync::Arc::new(vec![1, 2, 3])), transform);
    /// let data = mapped.data(); // transform runs once
    /// assert_eq!(data, Some(3));
    /// // use `data` freely below
    /// ```
    pub fn data(&self) -> Option<U> {
        self.source_data
            .as_ref()
            .map(|d| self.transform.apply(d.as_ref()))
    }

    pub fn has_data(&self) -> bool {
        self.source_data.is_some()
    }

    pub fn source_data(&self) -> Option<&T> {
        self.source_data.as_ref().map(|arc| arc.as_ref())
    }

    /// Refcount bump, no `T` clone. The returned `Arc` is owned, so the
    /// mapped borrow ends here and a later `entity.read_with` cannot nest.
    pub fn source_arc(&self) -> Option<Arc<T>> {
        self.source_data.clone()
    }

    /// The transform is not applied here; it runs lazily in [`data()`](Self::data).
    pub fn update_source(&mut self, data: Option<Arc<T>>) {
        self.source_data = data;
    }
}

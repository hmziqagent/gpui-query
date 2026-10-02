//! `Entity::read_with` returns `R` in older gpui but `C::Result<R>` (i.e.
//! `Result<R>` for `AsyncApp`) in 0.2.2. [`read_entity`] makes the closure
//! return `()` on both and captures the real value through a mutable local.

/// `None` when the entity could not be read (dropped, off-thread) under the `Result`-returning `read_with`.
#[inline]
pub(crate) fn read_entity<T: 'static, R, C: gpui::AppContext>(
    entity: &gpui::Entity<T>,
    cx: &C,
    f: impl FnOnce(&T, &gpui::App) -> R,
) -> Option<R> {
    let mut out: Option<R> = None;
    let _ = entity.read_with(cx, |value, app| {
        out = Some(f(value, app));
    });
    out
}

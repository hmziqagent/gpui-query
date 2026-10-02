/// Carries no state: bump sites call `cx.default_global::<CacheMutation>()`,
/// which pushes GPUI's `NotifyGlobalObservers` effect, and that notification
/// is what `observe_global` listeners (the `persist_with` driver) react to.
/// `default_global` seeds the marker on first bump, so bump sites never
/// panic.
#[derive(Default)]
pub struct CacheMutation;

impl gpui::Global for CacheMutation {}

use std::cell::Cell;

thread_local! {
    static RUNTIME_UTILIZATION: Cell<f64> = const { Cell::new(0.0) };
}

/// Returns the current runtime utilization estimate (0.0 to 1.0).
/// This is an EMA of the busy fraction of the single-threaded runtime.
pub fn current_utilization() -> f64 {
    RUNTIME_UTILIZATION.with(|u| u.get())
}

pub(crate) fn set_utilization(value: f64) {
    RUNTIME_UTILIZATION.with(|u| u.set(value));
}

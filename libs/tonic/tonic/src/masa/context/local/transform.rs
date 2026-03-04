#[cfg(all(feature = "prio_local_transform", not(feature = "prio_local")))]
compile_error!("Feature 'prio_local_transform' requires 'prio_local' to be enabled");

#[cfg(feature = "prio_local_transform")]
// Edit these constants between experiment runs to test different monotone transforms:
// w' = a * w^b
const REMAINING_ESTIMATE_TRANSFORM_A: f64 = 0.5;
#[cfg(feature = "prio_local_transform")]
const REMAINING_ESTIMATE_TRANSFORM_B: f64 = 1.0;

#[cfg(feature = "prio_local_transform")]
pub(super) fn transform_remaining_estimate(estimate: u64) -> u64 {
    let transformed =
        REMAINING_ESTIMATE_TRANSFORM_A * (estimate as f64).powf(REMAINING_ESTIMATE_TRANSFORM_B);
    if !transformed.is_finite() {
        return u64::MAX;
    }
    if transformed <= 0.0 {
        return 0;
    }
    if transformed >= u64::MAX as f64 {
        return u64::MAX;
    }
    transformed.round() as u64
}

#[cfg(not(feature = "prio_local_transform"))]
pub(super) fn transform_remaining_estimate(estimate: u64) -> u64 {
    estimate
}

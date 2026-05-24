// ══════════════════════════════════════════════════════════════════════════
// Wallclock-time decay for stale estimates
// ══════════════════════════════════════════════════════════════════════════

/// Wallclock-time decay constant (microseconds) for stale EMA estimates.
/// At age = TAU_DECAY_US, the contribution is multiplied by 1/e ≈ 0.37.
/// Picked at 5 s so that a sustained admission lockout breaks within tens of
/// seconds even without fresh observations.
pub(crate) const TAU_DECAY_US: f64 = 5_000_000.0;

/// Skip the `exp()` call when the most recent observation is younger than this.
/// At ages well below TAU_DECAY_US the decay factor is ≈ 1.0 anyway; avoiding
/// the transcendental keeps the hot path cheap.
pub(crate) const DECAY_THRESHOLD_US: u64 = 200_000;

/// `exp(-x)` for `x ≥ 0`, with an early-out for arguments large enough that
/// the result rounds to 0. For typical arguments this is just `f64::exp(-x)`,
/// ~10 ns on x86.
#[inline(always)]
pub(crate) fn fast_exp_neg(x: f64) -> f64 {
    debug_assert!(x >= 0.0);
    if x >= 50.0 {
        return 0.0;
    }
    (-x).exp()
}

/// Decay factor for an estimate whose most recent observation is `last_obs_us`
/// (microseconds since epoch), evaluated at `now_us`. Returns 1.0 for fresh
/// observations and shrinks toward 0 as the age grows past `TAU_DECAY_US`.
#[inline]
pub(crate) fn decay_factor(now_us: u64, last_obs_us: u64) -> f64 {
    let age = now_us.saturating_sub(last_obs_us);
    if age <= DECAY_THRESHOLD_US {
        1.0
    } else {
        fast_exp_neg(age as f64 / TAU_DECAY_US)
    }
}

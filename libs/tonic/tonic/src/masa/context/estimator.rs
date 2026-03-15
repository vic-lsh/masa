// Shared types and compile-time guards for latency estimation across all scheduling policies.

// --- Compile-time guards for estimator feature exclusivity ---

#[cfg(all(feature = "est_rms", feature = "est_hist"))]
compile_error!("Features 'est_rms' and 'est_hist' cannot be enabled simultaneously");

#[cfg(all(feature = "est_rms", feature = "est_mean_var"))]
compile_error!("Features 'est_rms' and 'est_mean_var' cannot be enabled simultaneously");

#[cfg(all(feature = "est_hist", feature = "est_mean_var"))]
compile_error!("Features 'est_hist' and 'est_mean_var' cannot be enabled simultaneously");

#[cfg(any(feature = "est_rms", feature = "est_hist", feature = "est_mean_var"))]
#[cfg(not(any(feature = "prio_local", feature = "adctl")))]
compile_error!(
    "Features 'est_rms', 'est_hist', or 'est_mean_var' require 'prio_local' or 'adctl' to be enabled"
);

// --- Default latency estimator type alias ---

/// Type alias for the latency estimator used across policies.
/// Selected at compile time via feature flags; defaults to LatencyRms.
#[cfg(feature = "est_hist")]
pub(crate) type DefaultLatencyEstimator = masa_core::LatencyDistribution;

#[cfg(feature = "est_mean_var")]
pub(crate) type DefaultLatencyEstimator = masa_core::LatencyMeanVar;

#[cfg(any(
    feature = "est_rms",
    all(
        not(feature = "est_rms"),
        not(feature = "est_hist"),
        not(feature = "est_mean_var")
    )
))]
pub(crate) type DefaultLatencyEstimator = masa_core::LatencyRms;

// --- Shared types ---

/// Identifies a parent→child method pair for latency tracking.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub(crate) struct ParentToChildId {
    pub parent_id: u64,
    pub child_id: u64,
}

impl ParentToChildId {
    pub(crate) fn to_key(&self) -> u64 {
        // Simple combination of two 32-bit (effective) IDs into one 64-bit key
        (self.parent_id << 32) | self.child_id
    }
}

impl std::fmt::Display for ParentToChildId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}=>{}", self.parent_id, self.child_id)
    }
}

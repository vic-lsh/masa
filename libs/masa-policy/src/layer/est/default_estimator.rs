// Shared types and compile-time guards for latency estimation across all scheduling policies.

// --- Compile-time guards for estimator feature exclusivity ---

#[cfg(all(feature = "est_rms", feature = "est_hist"))]
compile_error!("Features 'est_rms' and 'est_hist' cannot be enabled simultaneously");

#[cfg(all(feature = "est_rms", feature = "est_mean_var"))]
compile_error!("Features 'est_rms' and 'est_mean_var' cannot be enabled simultaneously");

#[cfg(all(feature = "est_hist", feature = "est_mean_var"))]
compile_error!("Features 'est_hist' and 'est_mean_var' cannot be enabled simultaneously");

#[cfg(any(feature = "est_rms", feature = "est_hist", feature = "est_mean_var"))]
#[cfg(not(feature = "estimator"))]
compile_error!(
    "Features 'est_rms', 'est_hist', or 'est_mean_var' require the 'estimator' feature to be enabled"
);

// --- Default latency estimator type alias ---

/// Type alias for the latency estimator used across policies.
/// Selected at compile time via feature flags; defaults to LatencyMeanVar.
#[cfg(feature = "est_hist")]
pub(crate) type DefaultLatencyEstimator = masa_core::LatencyDistribution;

#[cfg(feature = "est_rms")]
pub(crate) type DefaultLatencyEstimator = masa_core::LatencyRms;

#[cfg(any(
    feature = "est_mean_var",
    all(
        not(feature = "est_rms"),
        not(feature = "est_hist"),
        not(feature = "est_mean_var")
    )
))]
pub(crate) type DefaultLatencyEstimator = masa_core::LatencyMeanVar;

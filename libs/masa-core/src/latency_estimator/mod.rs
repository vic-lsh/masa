pub mod histogram;
pub mod mean_var;
pub mod rms;

/// Trait for latency estimators that can track latency values and provide estimates.
/// This allows different estimation strategies (e.g., histogram-based, RMS-based) to be used interchangeably.
pub trait LatencyEstimator: Send + Sync {
    /// Track a new latency value.
    fn track(&mut self, value: u64);

    /// Check if the estimator has enough data to provide an estimate.
    fn can_estimate(&self) -> bool;

    /// Get an estimate.
    /// The specific estimation strategy (e.g. which percentile to pick) is internal
    /// to the implementation.
    fn estimate(&self) -> u64;

    /// Get a conservative (mean-only, k=0) estimate, used for early-return thresholds.
    /// Using mean rather than mean+k*σ prevents over-aggressive shedding at low load.
    /// Default implementation returns estimate() for backwards compatibility.
    fn mean_estimate(&self) -> u64 {
        self.estimate()
    }

    /// Get an estimate using a dynamically supplied k value instead of the stored k.
    /// Returns mean + k_override * stddev. Default implementation ignores k_override
    /// and returns the standard estimate, for backwards compatibility.
    fn estimate_with_k(&self, _k_override: f64) -> u64 {
        self.estimate()
    }
}

pub use histogram::LatencyDistribution;
pub use mean_var::LatencyMeanVar;
pub use rms::LatencyRms;

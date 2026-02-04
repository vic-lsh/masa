pub mod histogram;
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
}

pub use histogram::LatencyDistribution;
pub use rms::LatencyRms;

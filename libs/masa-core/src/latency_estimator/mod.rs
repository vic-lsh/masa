pub mod histogram;
pub mod rms;

/// Trait for latency estimators that can track latency values and provide estimates.
/// This allows different estimation strategies (e.g., histogram-based, RMS-based) to be used interchangeably.
pub trait LatencyEstimator: Send + Sync {
    /// Track a new latency value.
    fn track(&mut self, value: u64);

    /// Check if the estimator has enough data to provide an estimate.
    fn can_estimate(&self) -> bool;

    /// Get an estimate for the given percentile.
    /// The percentile parameter is used by some estimators (e.g., histogram-based) to select
    /// a specific percentile value. Other estimators may ignore this parameter.
    fn estimate(&self, percentile: usize) -> u64;
}

pub use histogram::LatencyDistribution;
pub use rms::LatencyRms;

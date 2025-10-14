use anyhow::{Context, Result, anyhow};
use ordered_float::OrderedFloat;
use rand::Rng;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

/// How to interpolate between known percentile knots.
#[derive(Debug, Clone, Copy)]
pub enum InterpMode {
    /// Linear interpolation in percentile space (default, smooth).
    Linear,
    /// Step function using the lower knot (left-constant / floor).
    Lower,
    /// Step function using the upper knot (right-constant / ceil).
    Upper,
    /// Average of lower & upper knots when between points.
    Midpoint,
}

/// A latency quantile function represented by aligned knots:
///   ps: percentiles in [0, 1], strictly increasing
///   xs: latencies (ms) corresponding to each percentile
#[derive(Debug, Clone)]
pub struct Distribution {
    ps: Vec<f64>,
    xs: Vec<f64>,
    mode: InterpMode,
}

impl Distribution {
    /// Build from a percentile->latency map (percentiles given as strings like "99.5").
    /// Keys are parsed as f64 percent values in [0, 100]; duplicates keep the *last* value.
    pub fn from_percentile_map<K: AsRef<str>>(m: &HashMap<K, f64>) -> Result<Self> {
        if m.is_empty() {
            return Err(anyhow!("Empty percentile map"));
        }

        // Use OrderedFloat to get Ord implementation
        let mut tree: BTreeMap<OrderedFloat<f64>, f64> = BTreeMap::new();
        for (k, v) in m {
            let p = k
                .as_ref()
                .parse::<f64>()
                .with_context(|| format!("Failed to parse percentile key {:?}", k.as_ref()))?;
            tree.insert(OrderedFloat(p), *v);
        }

        let mut ps = Vec::with_capacity(tree.len());
        let mut xs = Vec::with_capacity(tree.len());
        for (p, x) in tree {
            ps.push((p.into_inner() / 100.0).clamp(0.0, 1.0));
            xs.push(x);
        }

        Ok(Self {
            ps,
            xs,
            mode: InterpMode::Linear,
        })
    }

    /// Choose a different interpolation mode.
    pub fn with_mode(mut self, mode: InterpMode) -> Self {
        self.mode = mode;
        self
    }

    /// Number of knots (percentile points) in this distribution.
    pub fn len(&self) -> usize {
        self.ps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ps.is_empty()
    }

    /// Quantile at percentile in [0, 100].
    pub fn quantile(&self, percentile: f64) -> f64 {
        self.quantile01((percentile / 100.0).clamp(0.0, 1.0))
    }

    /// Quantile at p in [0,1].
    pub fn quantile01(&self, p: f64) -> f64 {
        if self.ps.len() == 1 {
            return self.xs[0];
        }

        // Clamp to endpoints
        if p <= self.ps[0] {
            return self.xs[0];
        }
        if p >= *self.ps.last().unwrap() {
            return *self.xs.last().unwrap();
        }

        // Find right insertion point for p (first index with ps[idx] >= p)
        let idx = match self.ps.binary_search_by(|probe| {
            // Handle NaN safely: treat as less
            probe.partial_cmp(&p).unwrap_or(Ordering::Less)
        }) {
            Ok(i) => i,  // exact knot
            Err(i) => i, // in-between; i is upper index
        };

        if (self.ps[idx] - p).abs() < f64::EPSILON {
            // exactly at a knot
            return self.xs[idx];
        }

        // Interpolate between (lo .. hi)
        let hi = idx;
        let lo = idx - 1;
        let (p0, x0) = (self.ps[lo], self.xs[lo]);
        let (p1, x1) = (self.ps[hi], self.xs[hi]);

        match self.mode {
            InterpMode::Lower => x0,
            InterpMode::Upper => x1,
            InterpMode::Midpoint => 0.5 * (x0 + x1),
            InterpMode::Linear => {
                let t = (p - p0) / (p1 - p0);
                x0 + t * (x1 - x0)
            }
        }
    }

    /// Draw a single sample via inverse-transform sampling (U(0,1) → quantile).
    pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        let u: f64 = rng.random::<f64>(); // [0,1)
        self.quantile01(u)
    }

    /// Draw `n` samples.
    pub fn sample_n<R: Rng + ?Sized>(&self, rng: &mut R, n: usize) -> Vec<f64> {
        (0..n).map(|_| self.sample(rng)).collect()
    }

    /// Convenience: return (percentiles_in_0_1, latencies_ms) copies for inspection.
    pub fn knots(&self) -> (Vec<f64>, Vec<f64>) {
        (self.ps.clone(), self.xs.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn toy() -> Distribution {
        // 1% -> 10 ms, 50% -> 100 ms, 99% -> 1000 ms
        let m = HashMap::from([
            ("1".to_string(), 10.0),
            ("50".to_string(), 100.0),
            ("99".to_string(), 1000.0),
        ]);
        Distribution::from_percentile_map(&m).unwrap()
    }

    #[test]
    fn quantile_linear() {
        let d = toy().with_mode(InterpMode::Linear);
        assert!((d.quantile(1.0) - 10.0).abs() < 1e-9);
        assert!((d.quantile(50.0) - 100.0).abs() < 1e-9);
        // halfway in percentile space between 50 and 99:
        let q = d.quantile(74.5);
        // linear between 100 and 1000
        assert!(q > 500.0 && q < 600.0);
    }

    #[test]
    fn quantile_steps() {
        let d_lower = toy().with_mode(InterpMode::Lower);
        let d_upper = toy().with_mode(InterpMode::Upper);
        // Between 50 and 99:
        assert_eq!(d_lower.quantile(60.0), 100.0);
        assert_eq!(d_upper.quantile(60.0), 1000.0);
    }

    #[test]
    fn sampling_stable() {
        let d = toy();
        let mut rng = StdRng::seed_from_u64(42);
        let xs = d.sample_n(&mut rng, 5);
        assert_eq!(xs.len(), 5);
        // Values should lie within [min, max] knots
        let (min_x, max_x) = (10.0, 1000.0);
        assert!(xs.iter().all(|&x| x >= min_x && x <= max_x));
    }
}

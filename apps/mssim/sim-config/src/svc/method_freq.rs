use anyhow::{anyhow, Context, Result};
use rand::Rng;
use std::{collections::HashMap, path::PathBuf};

use crate::svc::MethodId;

/// File representation: { [service_name]: { [method_id]: freq, ... } , ... }
type RawInvokeFreq = HashMap<String, MethodInvokeFreq>;

type MethodInvokeFreq = HashMap<MethodId, u64>;

fn parse_invoke_freq(path: &PathBuf) -> Result<RawInvokeFreq> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read method frequency file: {:?}", path))?;

    let raw_map: RawInvokeFreq = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse method frequency file: {:?}", path))?;

    Ok(raw_map)
}

/// Errors you might encounter when building or using the sampler.
#[derive(Debug)]
pub enum SamplerError {
    Empty,
    ZeroTotal,
}

/// Sample items according to integer frequencies.
/// Construction is O(n); each sample is O(log n).
// TODO: make the sampling O(1)
struct WeightedSampler {
    keys: Vec<String>,
    /// Inclusive cumulative sums: cum[i] = sum_{j<=i} freq[j]
    cum: Vec<u64>,
    total: u64,
}

impl WeightedSampler {
    /// Build from a mapping of item -> frequency.
    pub fn from_map(map: &HashMap<String, u64>) -> Result<Self, SamplerError> {
        if map.is_empty() {
            return Err(SamplerError::Empty);
        }
        let mut keys = Vec::with_capacity(map.len());
        let mut cum = Vec::with_capacity(map.len());

        let mut running: u64 = 0;
        for (k, &f) in map.iter() {
            if f == 0 {
                // Zero weights are fine; they just don't change the running sum.
                // We still include the key to keep stable ordering with the map's iteration,
                // but it will never be picked unless all weights are zero.
            }
            keys.push(k.clone());
            running = running.saturating_add(f);
            cum.push(running);
        }

        if running == 0 {
            return Err(SamplerError::ZeroTotal);
        }

        Ok(Self {
            keys,
            cum,
            total: running,
        })
    }

    /// Total weight (sum of all frequencies).
    pub fn total_weight(&self) -> u64 {
        self.total
    }

    /// Number of distinct items.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Draw one sample. Returns a reference to the chosen key.
    pub fn sample<'a, R: Rng + ?Sized>(&'a self, rng: &mut R) -> &'a str {
        // pick x in [0, total-1]
        let x = rng.random_range(0..self.total);

        // Find the first index i s.t. cum[i] > x (upper_bound).
        let idx = self
            .cum
            .binary_search_by(|&c| {
                if c <= x {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            })
            .unwrap_err(); // binary_search_by returns Err(pos) for insertion point

        &self.keys[idx]
    }

    /// Draw `n` samples.
    #[allow(dead_code)]
    pub fn sample_many<R: Rng + ?Sized>(&self, rng: &mut R, n: usize) -> Vec<&str> {
        (0..n).map(|_| self.sample(rng)).collect()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(&key.to_string())
    }
}

pub struct MethodFreqSampler {
    sampler: WeightedSampler,
}

impl MethodFreqSampler {
    fn from_invoke_freq_map(freq_map: MethodInvokeFreq) -> Result<Self, SamplerError> {
        // Convert MethodId keys to String for the sampler.
        let str_map: HashMap<String, u64> = freq_map
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v))
            .collect();
        let sampler = WeightedSampler::from_map(&str_map)?;
        Ok(Self { sampler })
    }

    /// Draw one sample. Returns a reference to the chosen method id string.
    pub fn sample<'a, R: Rng + ?Sized>(&'a self, rng: &mut R) -> &'a str {
        self.sampler.sample(rng)
    }

    pub fn contains(&self, method: &str) -> bool {
        self.sampler.contains(method)
    }
}

pub struct MethodFreqMap {
    map: HashMap<String, MethodFreqSampler>,
}

impl MethodFreqMap {
    pub fn from_file_path(path: &PathBuf) -> Result<Self> {
        let raw_map = parse_invoke_freq(path)?;

        let mut map = HashMap::new();
        for (svc, freq_map) in raw_map.into_iter() {
            let sampler = MethodFreqSampler::from_invoke_freq_map(freq_map)
                .map_err(|_| anyhow!(format!("While building sampler for service {}", svc)))?;
            map.insert(svc, sampler);
        }

        Ok(Self { map })
    }

    pub fn get_service(&self, svc_name: &str) -> Option<&MethodFreqSampler> {
        self.map.get(svc_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        env!("CARGO_WORKSPACE_DIR").into()
    }

    #[test]
    fn test_parse_method_invoke_freq() {
        let svc_name = "MS_9287";
        let path =
            workspace_root().join("./trace-analysis/golden/S_32048416/interface_distribution.json");

        let map = MethodFreqMap::from_file_path(&path).expect("Parsing should not fail");

        let freq_map = map.get_service(svc_name).expect("Service should exist");

        // Raw data obtained from the golden file.
        let expected = vec![
            "29wNwTk-EQ:TDDL_QUERY",
            "ExNQLwkRHI:TDDL_QUERY",
            "OuyvbrayuW:TDDL_QUERY",
            "8aZ9IqfaWX:TDDL_QUERY",
        ];

        for method in expected {
            let method_id: MethodId = method.into();
            assert!(freq_map.contains(&method_id));
        }
    }

    #[test]
    fn test_weighted_sampler() {
        let freq_map: HashMap<String, u64> = vec![
            ("A".into(), 1),
            ("B".into(), 2),
            ("C".into(), 3),
            ("D".into(), 4),
        ]
        .into_iter()
        .collect();

        let sampler = WeightedSampler::from_map(&freq_map).expect("Should build sampler");

        assert_eq!(sampler.total_weight(), 10);
        assert_eq!(sampler.len(), 4);
        assert!(!sampler.is_empty());

        let mut rng = rand::rng();
        let samples = sampler.sample_many(&mut rng, 1000);

        // Check that all keys appear in the samples.
        let mut seen = HashMap::new();
        for s in samples {
            *seen.entry(s).or_insert(0) += 1;
        }

        for key in freq_map.keys() {
            assert!(
                seen.contains_key(key.as_str()),
                "Key {} should be sampled",
                key
            );
        }

        // check distribution
        assert!(seen["A"] < seen["B"]);
        assert!(seen["B"] < seen["C"]);
        assert!(seen["C"] < seen["D"]);
    }
}

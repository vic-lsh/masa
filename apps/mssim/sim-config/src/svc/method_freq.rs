use anyhow::{anyhow, Context, Result};
use rand::Rng;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::svc::{GraphId, MethodId, ServiceName};

type MethodInvokeFreq = HashMap<MethodId, u64>;
/// Service-level format: { service_name -> { method_id -> freq } }
type RawServiceInvokeFreq = HashMap<String, HashMap<String, u64>>;
/// Graph-oriented format: { graph_name -> { service_name -> { method_id -> freq } } }
/// Note: Keys are strings from JSON, will be converted to GraphId during parsing
type RawGraphInvokeFreq = HashMap<String, HashMap<String, HashMap<String, u64>>>;

/// Errors you might encounter when building or using the sampler.
#[derive(Debug)]
pub enum SamplerError {
    Empty,
    ZeroTotal,
}

/// Sample items according to integer frequencies.0.
/// Construction is O(n); each sample is O(log n).
// TODO: make the sampling O(1)
struct WeightedSampler {
    keys: Vec<MethodId>,
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
            keys.push(k.to_owned().into());
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

    /// Draw one sample. Returns the chosen key.
    pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MethodId {
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

        self.keys[idx].clone()
    }

    /// Draw `n` samples.
    #[allow(dead_code)]
    pub fn sample_many<R: Rng + ?Sized>(&self, rng: &mut R, n: usize) -> Vec<MethodId> {
        (0..n).map(|_| self.sample(rng)).collect()
    }

    pub fn contains(&self, key: &MethodId) -> bool {
        self.keys.contains(key)
    }
}

pub struct MethodFreqSampler {
    sampler: WeightedSampler,
}

impl MethodFreqSampler {
    fn from_invoke_freq_map(mut freq_map: MethodInvokeFreq) -> Result<Self, SamplerError> {
        // Remove zero-weight entries up front so we can detect empty maps accurately.
        freq_map.retain(|_, weight| *weight > 0);

        // Convert MethodId keys to String for the sampler.
        let str_map: HashMap<String, u64> = freq_map
            .into_iter()
            .map(|(k, v)| (k.into_owned(), v))
            .collect();
        let sampler = WeightedSampler::from_map(&str_map)?;
        Ok(Self { sampler })
    }

    /// Draw one sample. Returns the chosen method id.
    pub fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MethodId {
        self.sampler.sample(rng)
    }

    pub fn contains(&self, method: &MethodId) -> bool {
        self.sampler.contains(method)
    }
}

#[derive(Clone)]
pub struct SampledMethod {
    pub method: MethodId,
    pub graph: Option<GraphId>,
}

pub struct MethodFreqMap {
    by_graph: HashMap<GraphId, HashMap<ServiceName, MethodFreqSampler>>,
    aggregated: HashMap<ServiceName, MethodFreqSampler>,
}

impl MethodFreqMap {
    pub fn from_file_path(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read method frequency file: {:?}", path))?;

        // Extract graph name from directory path (e.g., "S_1823467" from "trace-analysis/graphs/S_1823467")
        let graph_name = GraphId::from_string(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "default_graph".to_string()),
        );

        // Parse as service-level format: { service_name -> { method_id -> freq } }
        let raw_service: RawServiceInvokeFreq = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse method frequency file: {:?} - expected {{ service: {{ method: frequency }} }}", path))?;

        // Convert to graph-level format by wrapping with graph name
        let mut raw_graph: RawGraphInvokeFreq = HashMap::new();
        raw_graph.insert(graph_name.as_str().to_string(), raw_service);

        Self::from_graph_map(raw_graph)
    }

    fn from_graph_map(raw_graph: RawGraphInvokeFreq) -> Result<Self> {
        let mut by_graph: HashMap<GraphId, HashMap<ServiceName, MethodFreqSampler>> =
            HashMap::new();
        let mut aggregated_raw: HashMap<ServiceName, HashMap<MethodId, u64>> = HashMap::new();

        for (graph_str, services) in raw_graph {
            let graph = GraphId::from_string(graph_str.clone());
            let mut service_map: HashMap<ServiceName, MethodFreqSampler> = HashMap::new();
            for (svc_raw, methods) in services {
                let svc = ServiceName::from_string(svc_raw);
                let mut freq_map: MethodInvokeFreq = HashMap::new();
                for (method, weight) in methods {
                    if weight == 0 {
                        continue;
                    }
                    let method_id: MethodId = method.into();
                    freq_map.insert(method_id.clone(), weight);
                    aggregated_raw
                        .entry(svc.clone())
                        .or_default()
                        .entry(method_id)
                        .and_modify(|existing| *existing = existing.saturating_add(weight))
                        .or_insert(weight);
                }

                if freq_map.is_empty() {
                    continue;
                }

                match MethodFreqSampler::from_invoke_freq_map(freq_map) {
                    Ok(sampler) => {
                        service_map.insert(svc.clone(), sampler);
                    }
                    Err(err) => {
                        return Err(anyhow!(format!(
                            "While building sampler for service {} in graph {}: {:?}",
                            svc,
                            graph.as_str(),
                            err
                        )));
                    }
                }
            }
            if !service_map.is_empty() {
                by_graph.insert(graph, service_map);
            }
        }

        let mut aggregated: HashMap<ServiceName, MethodFreqSampler> = HashMap::new();
        for (svc, freq_map) in aggregated_raw {
            match MethodFreqSampler::from_invoke_freq_map(freq_map) {
                Ok(sampler) => {
                    aggregated.insert(svc, sampler);
                }
                Err(err) => {
                    return Err(anyhow!(format!(
                        "While building aggregated sampler for service {}: {:?}",
                        svc, err
                    )));
                }
            }
        }

        Ok(Self {
            by_graph,
            aggregated,
        })
    }

    pub fn get_service(&self, svc_name: &ServiceName) -> Option<&MethodFreqSampler> {
        self.aggregated.get(svc_name)
    }

    pub fn sample_method<R: Rng + ?Sized>(
        &self,
        svc_name: &ServiceName,
        graph_hint: &GraphId,
        rng: &mut R,
    ) -> Option<SampledMethod> {
        if let Some(method) = self.sample_from_graph(graph_hint, svc_name, rng) {
            return Some(SampledMethod {
                method,
                graph: Some(graph_hint.clone()),
            });
        }

        self.aggregated
            .get(svc_name)
            .map(|sampler| SampledMethod {
                method: sampler.sample(rng),
                graph: None,
            })
            .or_else(|| {
                self.by_graph
                    .values()
                    .find_map(|services| self.sample_from_services_map(services, svc_name, rng))
            })
    }

    pub fn contains_service_in_graph(&self, graph: &GraphId, svc_name: &ServiceName) -> bool {
        self.by_graph
            .get(graph)
            .map_or(false, |services| services.contains_key(svc_name))
    }

    fn sample_from_graph<R: Rng + ?Sized>(
        &self,
        graph: &GraphId,
        svc_name: &ServiceName,
        rng: &mut R,
    ) -> Option<MethodId> {
        self.by_graph
            .get(graph)
            .and_then(|services| services.get(svc_name))
            .map(|sampler| sampler.sample(rng))
    }

    fn sample_from_services_map<R: Rng + ?Sized>(
        &self,
        services: &HashMap<ServiceName, MethodFreqSampler>,
        svc_name: &ServiceName,
        rng: &mut R,
    ) -> Option<SampledMethod> {
        services.get(svc_name).map(|sampler| SampledMethod {
            method: sampler.sample(rng),
            graph: None,
        })
    }

    /// Merge another MethodFreqMap into this one.
    /// Combines by_graph maps and updates aggregated samplers.
    pub fn merge(&mut self, other: MethodFreqMap) {
        // Merge by_graph maps
        for (graph_name, services) in other.by_graph {
            self.by_graph.insert(graph_name, services);
        }

        // For simplicity, if other has aggregated data we don't have, add it
        // This is a simplified merge - a full merge would require extracting frequencies
        for (svc, sampler) in other.aggregated {
            if !self.aggregated.contains_key(&svc) {
                self.aggregated.insert(svc, sampler);
            }
        }
    }

    /// Create a new MethodFreqMap by merging multiple maps.
    pub fn merge_maps(maps: Vec<MethodFreqMap>) -> Result<Self> {
        if maps.is_empty() {
            return Ok(MethodFreqMap {
                by_graph: HashMap::new(),
                aggregated: HashMap::new(),
            });
        }

        let result = maps.into_iter().next().unwrap();
        // Note: Full merge would require extracting and combining frequencies
        // For now, this keeps the first map's aggregated data
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn parse_graph_method_invoke_freq() {
        let temp = TempDir::new().expect("create temp dir");
        let graph_dir = temp.path().join("graph_alpha");
        fs::create_dir_all(&graph_dir).expect("create graph dir");
        let path = graph_dir.join("interface_distribution.json");
        let mut tmp = fs::File::create(&path).expect("create temp file");
        writeln!(
            tmp,
            r#"{{
  "svc_one": {{
    "method_a": 10,
    "method_b": 5,
    "method_c": 7
  }}
}}"#
        )
        .expect("write json");

        let map = MethodFreqMap::from_file_path(&path).expect("parse service map");

        let svc = ServiceName::from_string("svc_one".into());
        let freq_map = map.get_service(&svc).expect("service aggregated sampler");

        let expected = vec!["method_a", "method_b", "method_c"];
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
            let method_id: MethodId = key.clone().into();
            assert!(
                seen.contains_key(&method_id),
                "Key {} should be sampled",
                key
            );
        }

        // check distribution
        let a: MethodId = "A".into();
        let b: MethodId = "B".into();
        let c: MethodId = "C".into();
        let d: MethodId = "D".into();
        assert!(seen[&a] < seen[&b]);
        assert!(seen[&b] < seen[&c]);
        assert!(seen[&c] < seen[&d]);
    }

    #[test]
    fn graph_shape_sampling_prefers_graph_hint() {
        let temp = TempDir::new().expect("create temp dir");
        let graph_dir = temp.path().join("graph_b");
        fs::create_dir_all(&graph_dir).expect("create graph dir");
        let path = graph_dir.join("interface_distribution.json");
        let mut tmp = fs::File::create(&path).expect("create temp file");
        writeln!(
            tmp,
            r#"{{
  "svc_one": {{
    "method_c": 7
  }},
  "svc_two": {{
    "method_d": 3
  }}
}}"#
        )
        .expect("write json");

        let map = MethodFreqMap::from_file_path(&path).expect("parse service map");

        let svc_one = ServiceName::from_string("svc_one".into());
        let mut rng = rand::rng();
        let graph_b = GraphId::from_string("graph_b".to_string());
        let sampled = map
            .sample_method(&svc_one, &graph_b, &mut rng)
            .expect("graph b sampling");
        assert_eq!(sampled.graph.as_ref().map(|g| g.as_str()), Some("graph_b"));
        assert_eq!(sampled.method.as_ref(), "method_c");

        let svc_two = ServiceName::from_string("svc_two".into());
        let missing_graph = GraphId::from_string("missing_graph".to_string());
        let sampled_two = map
            .sample_method(&svc_two, &missing_graph, &mut rng)
            .expect("fallback to aggregated");
        assert_eq!(sampled_two.graph, None);
        assert_eq!(sampled_two.method.as_ref(), "method_d");
    }
}

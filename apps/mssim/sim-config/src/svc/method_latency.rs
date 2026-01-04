use crate::svc::{GraphId, MethodId, ServiceName};
use anyhow::{Context, Result, anyhow};
use std::{collections::HashMap, fs, path::PathBuf};

use crate::dist::Distribution;

/// Stores latency distributions keyed by graph along with a graph-agnostic fallback.
#[derive(Debug)]
pub struct MethodLatencyDistMap {
    by_graph: HashMap<GraphId, HashMap<MethodId, Distribution>>,
}

/// Raw service-level format: { service: { method: { percentile: latency } } }
type RawServiceShape = HashMap<String, HashMap<String, HashMap<String, f64>>>;
/// Raw graph shape: { graph: { service: { method: { percentile: latency } } } }
type RawGraphShape = HashMap<String, HashMap<String, HashMap<String, HashMap<String, f64>>>>;

impl MethodLatencyDistMap {
    pub fn from_file_path(path: &PathBuf, service_name: ServiceName) -> Result<Self> {
        let config_str = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;

        // Extract graph name from directory path (e.g., "S_1823467" from "trace-analysis/graphs/S_1823467")
        let graph_name = GraphId::from_string(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "default_graph".to_string()),
        );

        Self::from_str(&config_str, service_name, graph_name)
    }

    fn from_str(config_str: &str, service_name: ServiceName, graph_name: GraphId) -> Result<Self> {
        // Parse as service-level format: { service: { method: { percentile: latency } } }
        let raw_service: RawServiceShape = serde_json::from_str(config_str)
            .context("Invalid JSON for latency config - expected { service: { method: { percentile: latency } } }")?;

        // Convert to graph-level format by wrapping with graph name
        let mut raw: RawGraphShape = HashMap::new();
        raw.insert(graph_name.as_str().to_string(), raw_service);

        Self::from_graph_shape(raw, service_name)
    }

    fn from_graph_shape(mut raw: RawGraphShape, service_name: ServiceName) -> Result<Self> {
        let mut by_graph: HashMap<GraphId, HashMap<MethodId, Distribution>> = HashMap::new();
        let mut graphs_for_service: Vec<GraphId> = Vec::new();

        let mut entries: Vec<_> = raw.drain().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (graph_name_str, services) in entries {
            let graph_name = GraphId::from_string(graph_name_str);
            for (svc_raw, methods) in services {
                let svc = ServiceName::from_string(svc_raw);
                if svc != service_name {
                    continue;
                }

                let mut method_map = HashMap::new();
                for (method, p2l) in methods {
                    let dist = Distribution::from_percentile_map(&p2l).with_context(|| {
                        format!(
                            "While parsing {svc}.{method} in graph {}",
                            graph_name.as_str()
                        )
                    })?;
                    let method_id: MethodId = method.into();
                    method_map.insert(method_id, dist);
                }

                if method_map.is_empty() {
                    continue;
                }

                graphs_for_service.push(graph_name.clone());
                by_graph.insert(graph_name.clone(), method_map);
            }
        }

        if by_graph.is_empty() {
            return Err(anyhow!(format!(
                "Service name {} doesn't exist in latency graph config",
                service_name
            )));
        }

        graphs_for_service.sort();
        graphs_for_service.dedup();

        Ok(MethodLatencyDistMap { by_graph })
    }

    pub fn get_method_dist(
        &self,
        method: &MethodId,
        graph_name: &GraphId,
    ) -> Option<&Distribution> {
        if let Some(dist) = self
            .by_graph
            .get(graph_name)
            .and_then(|methods| methods.get(method))
        {
            return Some(dist);
        }

        None
    }

    /// Merge another MethodLatencyDistMap into this one.
    /// If the same graph exists in both, the other map's data takes precedence.
    pub fn merge(&mut self, other: MethodLatencyDistMap) {
        for (graph_name, methods) in other.by_graph {
            self.by_graph.insert(graph_name, methods);
        }
    }

    /// Create a new MethodLatencyDistMap by merging multiple maps.
    pub fn merge_maps(maps: Vec<MethodLatencyDistMap>) -> Self {
        let mut result = MethodLatencyDistMap {
            by_graph: HashMap::new(),
        };
        for map in maps {
            result.merge(map);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn test_parsing() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let graph_dir = temp.path().join("graph_alpha");
        fs::create_dir_all(&graph_dir).expect("create graph dir");
        let path = graph_dir.join("latency_percentiles.json");
        let mut tmp = fs::File::create(&path).expect("create temp file");
        writeln!(
            tmp,
            r#"{{
  "svc-one": {{
    "method_a": {{
      "50": 5.0,
      "99": 9.0
    }}
  }}
}}"#
        )
        .expect("write json");

        let svc_name = ServiceName::from_string("svc_one".into());
        let dist_map = MethodLatencyDistMap::from_file_path(&path, svc_name.clone())
            .expect("Parsing should not fail");

        let method: MethodId = "method_a".into();
        let graph_id = GraphId::from_string("graph_alpha".to_string());
        let dist = dist_map
            .get_method_dist(&method, &graph_id)
            .expect("Method distribution should exist");

        assert!(dist.quantile(50.0) >= 5.0);
        assert!(dist.quantile(99.0) >= 9.0);
    }

    #[test]
    fn graph_shape_parsing() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let graph_dir = temp.path().join("graph_alpha");
        fs::create_dir_all(&graph_dir).expect("create graph dir");
        let path = graph_dir.join("latency_percentiles.json");
        let mut tmp = fs::File::create(&path).expect("create temp file");
        writeln!(
            tmp,
            r#"{{
  "svc-one": {{
    "method_a": {{
      "50": 5.0
    }},
    "method_b": {{
      "50": 6.0
    }}
  }}
}}"#
        )
        .expect("write json");

        let svc = ServiceName::from_string("svc_one".into());
        let map =
            MethodLatencyDistMap::from_file_path(&path, svc.clone()).expect("parse graph latency");

        let graph_id = GraphId::from_string("graph_alpha".to_string());
        let method_a: MethodId = "method_a".into();
        let dist_a = map
            .get_method_dist(&method_a, &graph_id)
            .expect("graph alpha method");
        assert_eq!(dist_a.quantile(50.0), 5.0);

        let method_b: MethodId = "method_b".into();
        let dist_b = map
            .get_method_dist(&method_b, &graph_id)
            .expect("fallback to primary graph");
        assert_eq!(dist_b.quantile(50.0), 6.0);
    }
}

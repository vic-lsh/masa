use crate::svc::{MethodId, ServiceName};
use anyhow::{Context, Result, anyhow};
use std::{collections::HashMap, fs, path::PathBuf};

use crate::dist::Distribution;

/// Stores latency distributions keyed by graph along with a graph-agnostic fallback.
#[derive(Debug)]
pub struct MethodLatencyDistMap {
    by_graph: HashMap<String, HashMap<MethodId, Distribution>>,
    fallback: HashMap<MethodId, Distribution>,
    primary_graph: Option<String>,
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
        let graph_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "default_graph".to_string());
        
        Self::from_str(&config_str, service_name, graph_name)
    }

    fn from_str(config_str: &str, service_name: ServiceName, graph_name: String) -> Result<Self> {
        // Parse as service-level format: { service: { method: { percentile: latency } } }
        let raw_service: RawServiceShape = serde_json::from_str(config_str)
            .context("Invalid JSON for latency config - expected { service: { method: { percentile: latency } } }")?;
        
        // Convert to graph-level format by wrapping with graph name
        let mut raw: RawGraphShape = HashMap::new();
        raw.insert(graph_name, raw_service);
        
        Self::from_graph_shape(raw, service_name)
    }

    fn from_graph_shape(mut raw: RawGraphShape, service_name: ServiceName) -> Result<Self> {
        let mut by_graph: HashMap<String, HashMap<MethodId, Distribution>> = HashMap::new();
        let mut fallback: HashMap<MethodId, Distribution> = HashMap::new();
        let mut graphs_for_service: Vec<String> = Vec::new();

        let mut entries: Vec<_> = raw.drain().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (graph_name, services) in entries {
            for (svc_raw, methods) in services {
                let svc = ServiceName::from_string(svc_raw);
                if svc != service_name {
                    continue;
                }

                let mut method_map = HashMap::new();
                for (method, p2l) in methods {
                    let dist = Distribution::from_percentile_map(&p2l).with_context(|| {
                        format!("While parsing {svc}.{method} in graph {graph_name}")
                    })?;
                    let method_id: MethodId = method.into();
                    fallback
                        .entry(method_id.clone())
                        .or_insert_with(|| dist.clone());
                    method_map.insert(method_id, dist);
                }

                if method_map.is_empty() {
                    continue;
                }

                graphs_for_service.push(graph_name.clone());
                by_graph.insert(graph_name.clone(), method_map);
            }
        }

        if by_graph.is_empty() && fallback.is_empty() {
            return Err(anyhow!(format!(
                "Service name {} doesn't exist in latency graph config",
                service_name
            )));
        }

        graphs_for_service.sort();
        graphs_for_service.dedup();
        let primary_graph = graphs_for_service.first().cloned();

        Ok(MethodLatencyDistMap {
            by_graph,
            fallback,
            primary_graph,
        })
    }

    pub fn get_method_dist(
        &self,
        method: &MethodId,
        graph_hint: Option<&str>,
    ) -> Option<&Distribution> {
        if let Some(graph) = graph_hint {
            if let Some(dist) = self
                .by_graph
                .get(graph)
                .and_then(|methods| methods.get(method))
            {
                return Some(dist);
            }
        }

        if let Some(primary) = self.primary_graph() {
            if let Some(dist) = self
                .by_graph
                .get(primary)
                .and_then(|methods| methods.get(method))
            {
                return Some(dist);
            }
        }

        self.fallback.get(method).or_else(|| {
            self.by_graph
                .values()
                .find_map(|methods| methods.get(method))
        })
    }

    pub fn primary_graph(&self) -> Option<&str> {
        self.primary_graph.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parsing() {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp file");
        writeln!(
            tmp,
            r#"{{
  "graph_alpha": {{
    "svc-one": {{
      "method_a": {{
        "50": 5.0,
        "99": 9.0
      }}
    }}
  }}
}}"#
        )
        .expect("write json");

        let svc_name = ServiceName::from_string("svc_one".into());
        let dist_map =
            MethodLatencyDistMap::from_file_path(&tmp.path().to_path_buf(), svc_name.clone())
                .expect("Parsing should not fail");

        let method: MethodId = "method_a".into();
        let dist = dist_map
            .get_method_dist(&method, Some("graph_alpha"))
            .expect("Method distribution should exist");

        assert!(dist.quantile(50.0) >= 5.0);
        assert!(dist.quantile(99.0) >= 9.0);
    }

    #[test]
    fn graph_shape_parsing() {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp file");
        writeln!(
            tmp,
            r#"{{
  "graph_alpha": {{
    "svc-one": {{
      "method_a": {{
        "50": 5.0
      }}
    }}
  }},
  "graph_beta": {{
    "svc-one": {{
      "method_b": {{
        "50": 6.0
      }}
    }}
  }}
}}"#
        )
        .expect("write json");

        let svc = ServiceName::from_string("svc_one".into());
        let map = MethodLatencyDistMap::from_file_path(&tmp.path().to_path_buf(), svc.clone())
            .expect("parse graph latency");

        assert_eq!(map.primary_graph(), Some("graph_alpha"));

        let method_a: MethodId = "method_a".into();
        let dist_a = map
            .get_method_dist(&method_a, Some("graph_alpha"))
            .expect("graph alpha method");
        assert_eq!(dist_a.quantile(50.0), 5.0);

        let method_b: MethodId = "method_b".into();
        let dist_b = map
            .get_method_dist(&method_b, None)
            .expect("fallback to other graph");
        assert_eq!(dist_b.quantile(50.0), 6.0);
    }
}

use std::{borrow::Cow, fmt::Display, path::PathBuf};

pub mod call_graph;
pub mod call_sequence;
pub mod method_freq;
pub mod method_latency;

use masa::MethodId;
use method_latency::MethodLatencyDistMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceName(Cow<'static, str>);

impl Display for ServiceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<'de> Deserialize<'de> for ServiceName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(ServiceName::from_string(s))
    }
}

impl Serialize for ServiceName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl ServiceName {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self::from_string(s.into())
    }

    pub fn from_string(s: String) -> Self {
        ServiceName(Cow::Owned(Self::format_svc_name(s)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn format_svc_name(s: String) -> String {
        // Format the name such that it is a legal docker container name
        let formatted = s.replace('_', "-").to_lowercase();
        formatted
    }
}

impl Into<String> for ServiceName {
    fn into(self) -> String {
        self.0.into_owned()
    }
}

impl Into<String> for &ServiceName {
    fn into(self) -> String {
        self.0.to_owned().into_owned()
    }
}

pub struct CallGraphConfig {
    pub method_latency: Option<MethodLatencyDistMap>,
    pub method_freq_map: Option<method_freq::MethodFreqMap>,
    pub call_graph: call_graph::CallGraph,
}

impl CallGraphConfig {
    /// Load config from call graph directories.
    /// Unions call graphs and merges latency/frequency maps.
    pub fn from_callgraph_dirs(
        dirs: &[PathBuf],
        svc_name: &ServiceName,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Load and union call graphs from all directories
        let mut call_graphs = Vec::new();
        for dir in dirs {
            let call_graph_path = dir.join("edges.csv");
            let call_graph = call_graph::CallGraph::from_path(call_graph_path)?;
            call_graphs.push(call_graph);
        }

        // Union all call graphs
        let unioned_call_graph = call_graph::CallGraph::union(call_graphs);

        // Load and merge method latency from all directories
        let mut all_latency_maps = Vec::new();
        for dir in dirs {
            let method_latency_path = dir.join("latency_percentiles.json");
            if method_latency_path.exists() {
                match MethodLatencyDistMap::from_file_path(&method_latency_path, svc_name.clone()) {
                    Ok(map) => all_latency_maps.push(map),
                    Err(_) => continue, // Skip if this graph doesn't have latency for this service
                }
            }
        }

        let method_latency = if all_latency_maps.is_empty() {
            None
        } else {
            // Merge all latency maps
            Some(MethodLatencyDistMap::merge_maps(all_latency_maps))
        };

        // Load and merge method frequency from all directories
        let mut all_freq_maps = Vec::new();
        for dir in dirs {
            let method_freq_path = dir.join("interface_distribution.json");
            if method_freq_path.exists() {
                match method_freq::MethodFreqMap::from_file_path(&method_freq_path) {
                    Ok(map) => all_freq_maps.push(map),
                    Err(_) => continue,
                }
            }
        }

        let method_freq_map = if all_freq_maps.is_empty() {
            None
        } else {
            // Merge frequency maps
            method_freq::MethodFreqMap::merge_maps(all_freq_maps).ok()
        };

        Ok(CallGraphConfig {
            call_graph: unioned_call_graph,
            method_latency,
            method_freq_map,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_read_config() {
        let temp = TempDir::new().expect("create temp dir");
        let dir = temp.path().join("graph_main");
        fs::create_dir_all(&dir).expect("create graph dir");

        let edges_path = dir.join("edges.csv");
        let mut edges_file = fs::File::create(&edges_path).expect("create edges");
        writeln!(edges_file, "caller,callee,weight").unwrap();
        writeln!(edges_file, "svc_alpha,svc_beta,2").unwrap();

        fs::write(
            dir.join("latency_percentiles.json"),
            r#"{
  "svc_alpha": {
    "method_x": {
      "50": 5.0,
      "99": 9.0
    }
  }
}"#,
        )
        .expect("write latency");

        fs::write(
            dir.join("interface_distribution.json"),
            r#"{
  "svc_alpha": {
    "method_x": 10,
    "method_y": 5,
    "method_z": 7
  }
}"#,
        )
        .expect("write interface distribution");

        let svc_name = ServiceName::from_string("svc_alpha".into());

        let config = CallGraphConfig::from_callgraph_dirs(&[dir.to_path_buf()], &svc_name)
            .expect("Reading should not fail");

        assert!(config.call_graph.callees_of(&svc_name).len() > 0);

        let method = "method_x".into();
        let dist = config
            .method_latency
            .as_ref()
            .expect("Method latency should be present")
            .get_method_dist(&method, "graph_main")
            .expect("Method distribution should exist");

        let p50 = dist.sample(&mut rand::rng());
        assert!(p50 > 0.0);

        let freq_map = config.method_freq_map.as_ref().unwrap();
        let mut rng = rand::rng();
        let sampled = freq_map
            .sample_method(&svc_name, "graph_main", &mut rng)
            .expect("service must have methods");
        assert!(["method_x", "method_y", "method_z"].contains(&sampled.method.as_ref()));
    }
}

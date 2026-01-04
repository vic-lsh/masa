use std::{borrow::Cow, fmt::Display, path::PathBuf};

pub mod call_graph;
pub mod call_sequence;
pub mod method_freq;
pub mod method_latency;

use masa::MethodId;
use method_latency::MethodLatencyDistMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphId(Cow<'static, str>);

impl Display for GraphId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl<'de> Deserialize<'de> for GraphId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        Ok(GraphId::from_string(s))
    }
}

impl Serialize for GraphId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl GraphId {
    pub fn new<S: Into<String>>(s: S) -> Self {
        Self::from_string(s.into())
    }

    pub fn from_string(s: String) -> Self {
        GraphId(Cow::Owned(Self::normalize_graph_id(s)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn normalize_graph_id(s: String) -> String {
        // Normalize to canonical format: "S_XXXXX" (uppercase S, underscore)
        // Handle input formats: "s-XXXXX", "S_XXXXX", "s_XXXXX", etc.
        let mut normalized = s;

        // Convert "s-" prefix to "S_"
        if normalized.starts_with("s-") {
            normalized = normalized.replacen("s-", "S_", 1);
        } else if normalized.starts_with("s_") {
            normalized = normalized.replacen("s_", "S_", 1);
        } else if normalized.starts_with("S-") {
            normalized = normalized.replacen("S-", "S_", 1);
        }

        // Convert all remaining hyphens to underscores
        normalized = normalized.replace('-', "_");

        normalized
    }
}

impl Into<String> for GraphId {
    fn into(self) -> String {
        self.0.into_owned()
    }
}

impl Into<String> for &GraphId {
    fn into(self) -> String {
        self.0.to_owned().into_owned()
    }
}

pub struct CallGraphConfig {
    pub method_latency: Option<MethodLatencyDistMap>,
    pub method_freq_map: Option<method_freq::MethodFreqMap>,
    pub call_graph: call_graph::CallGraph,
    /// Call sequences for this service, keyed by graph_id
    pub call_sequences: HashMap<GraphId, Option<call_sequence::CallSequence>>,
    /// USER call sequences, keyed by graph_id
    pub user_call_sequences: HashMap<GraphId, call_sequence::CallSequence>,
}

/// Enumerates all subdirectories under a base directory.
/// Returns a sorted list of subdirectories, or an error if the base directory doesn't exist,
/// cannot be read, or contains no subdirectories.
fn enumerate_callgraph_dirs(
    base_dir: &PathBuf,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut callgraph_dirs: Vec<PathBuf> = Vec::new();

    if base_dir.exists() {
        // Enumerate all directories under the callgraphs base directory
        match std::fs::read_dir(base_dir) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.is_dir() {
                            callgraph_dirs.push(path);
                        }
                    }
                }
            }
            Err(e) => {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("Failed to read {}: {}", base_dir.display(), e),
                )));
            }
        }
    } else {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "Callgraphs base directory does not exist: {}",
                base_dir.display()
            ),
        )));
    }

    if callgraph_dirs.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "No call graph directories found. Expected {}/*",
                base_dir.display()
            ),
        )));
    }

    // Sort for consistent ordering
    callgraph_dirs.sort();
    Ok(callgraph_dirs)
}

impl CallGraphConfig {
    /// Load config from a base directory containing multiple call graph subdirectories.
    /// Enumerates all subdirectories under the base directory, sorts them, and loads config from all of them.
    pub fn from_multi_callgraph_dir(
        base_dir: &PathBuf,
        svc_name: &ServiceName,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let callgraph_dirs = enumerate_callgraph_dirs(base_dir)?;

        println!(
            "Loading config from {} call graph directory(ies)",
            callgraph_dirs.len()
        );
        for (i, dir) in callgraph_dirs.iter().enumerate() {
            println!("  [{}] {}", i + 1, dir.display());
        }

        let config = Self::from_callgraph_dirs(&callgraph_dirs, svc_name)?;
        Ok(config)
    }

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

        // Load call sequences from all directories
        let mut call_sequences: HashMap<GraphId, Option<call_sequence::CallSequence>> =
            HashMap::new();
        let mut user_call_sequences: HashMap<GraphId, call_sequence::CallSequence> = HashMap::new();

        for callgraph_dir in dirs {
            // Extract graph_id from directory name (e.g., "S_14677443" from "/app/callgraphs/S_14677443")
            let graph_id = GraphId::from_string(
                callgraph_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "default".to_string()),
            );

            // Try loading call sequence for this graph
            if let Ok(Some(call_sequence)) =
                call_sequence::load_call_sequence(callgraph_dir, svc_name, Some(&graph_id))
            {
                call_sequences.insert(graph_id.clone(), Some(call_sequence));
            } else if let Ok(Some(call_sequence)) =
                call_sequence::load_call_sequence(callgraph_dir, svc_name, None)
            {
                // Fallback: try without graph_id (backward compatibility)
                call_sequences.insert(graph_id.clone(), Some(call_sequence));
            }

            // Try loading USER call sequence for this graph
            if let Ok(user_call_sequence) =
                call_sequence::load_root_user_call_sequence(callgraph_dir, Some(&graph_id))
            {
                user_call_sequences.insert(graph_id.clone(), user_call_sequence);
            } else if let Ok(user_call_sequence) =
                call_sequence::load_root_user_call_sequence(callgraph_dir, None)
            {
                // Fallback: try without graph_id
                user_call_sequences.insert(graph_id.clone(), user_call_sequence);
            }
        }


        Ok(CallGraphConfig {
            call_graph: unioned_call_graph,
            method_latency,
            method_freq_map,
            call_sequences,
            user_call_sequences,
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
        let graph_id = GraphId::from_string("graph_main".to_string());
        let dist = config
            .method_latency
            .as_ref()
            .expect("Method latency should be present")
            .get_method_dist(&method, &graph_id)
            .expect("Method distribution should exist");

        let p50 = dist.sample(&mut rand::rng());
        assert!(p50 > 0.0);

        let freq_map = config.method_freq_map.as_ref().unwrap();
        let mut rng = rand::rng();
        let sampled = freq_map
            .sample_method(&svc_name, &graph_id, &mut rng)
            .expect("service must have methods");
        assert!(["method_x", "method_y", "method_z"].contains(&sampled.method.as_ref()));
    }
}

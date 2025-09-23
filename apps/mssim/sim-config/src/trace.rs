use std::path::PathBuf;

use crate::svc::{call_graph, method_freq};

pub struct TraceConfig {
    pub method_freq_map: method_freq::MethodFreqMap,
    pub call_graph: call_graph::CallGraph,
}

impl TraceConfig {
    pub fn from_config_dir(dir: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let call_graph_path = dir.join("edges.csv");
        let call_graph = call_graph::CallGraph::from_path(call_graph_path)?;

        let method_freq_path = dir.join("interface_distribution.json");
        let method_freq = method_freq::MethodFreqMap::from_file_path(&method_freq_path)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(TraceConfig {
            call_graph,
            method_freq_map: method_freq,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        env!("CARGO_WORKSPACE_DIR").into()
    }

    #[test]
    fn test_read_config() {
        let path = workspace_root().join("./trace-analysis/golden/S_32048416/");

        let config = TraceConfig::from_config_dir(&path).expect("Reading should not fail");

        let svc_name = "MS_49817";

        assert!(config.call_graph.callees_of(svc_name).len() > 0);

        let sampled_method = config
            .method_freq_map
            .get_service(&svc_name)
            .expect("service must exist")
            .sample(&mut rand::rng());
        assert!(!sampled_method.is_empty());

        let valid_methods = vec!["wZa2gEnTxC", "daq6sEhEBy"];
        assert!(valid_methods.contains(&sampled_method));
    }
}

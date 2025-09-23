use std::{borrow::Cow, path::PathBuf};

pub mod call_graph;
pub mod method_freq;
pub mod method_latency;

pub use method_latency::MethodLatencyDistMap;

pub type MethodId = Cow<'static, str>;

pub struct ServiceTraceConfig {
    pub method_latency: MethodLatencyDistMap,
    pub method_freq: method_freq::MethodFreqSampler,
    pub call_graph: call_graph::CallGraph,
}

impl ServiceTraceConfig {
    pub fn read_config_dir(
        dir: &PathBuf,
        svc_name: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let call_graph_path = dir.join("edges.csv");
        let call_graph = call_graph::CallGraph::from_path(call_graph_path)?;

        let method_latency_path = dir.join("latency_percentiles.json");
        let method_latency =
            MethodLatencyDistMap::from_file_path(&method_latency_path, svc_name)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let method_freq_path = dir.join("interface_distribution.json");
        let method_freq =
            method_freq::MethodFreqSampler::from_config_path(&method_freq_path, svc_name)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(ServiceTraceConfig {
            call_graph,
            method_latency,
            method_freq,
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
        let svc_name = "MS_49817";
        let path = workspace_root().join("./trace-analysis/golden/S_32048416/");

        let config =
            ServiceTraceConfig::read_config_dir(&path, svc_name).expect("Reading should not fail");

        assert!(config.call_graph.callees_of(svc_name).len() > 0);

        let method = "daq6sEhEBy".into();
        let dist = config
            .method_latency
            .get_method_dist(&method)
            .expect("Method distribution should exist");

        let p50 = dist.sample(&mut rand::rng());
        assert!(p50 > 0.0);

        let sampled_method = config.method_freq.sample(&mut rand::rng());
        assert!(!sampled_method.is_empty());

        let valid_methods = vec!["wZa2gEnTxC", "daq6sEhEBy"];
        assert!(valid_methods.contains(&sampled_method));
    }
}

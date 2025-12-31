use std::path::PathBuf;

use crate::svc::{call_graph, method_freq};

pub struct TraceConfig {
    pub method_freq_map: Option<method_freq::MethodFreqMap>,
    pub call_graph: call_graph::CallGraph,
}

impl TraceConfig {
    pub fn from_config_dir(dir: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let call_graph_path = dir.join("edges.csv");
        let call_graph = call_graph::CallGraph::from_path(call_graph_path)?;

        let method_freq_path = dir.join("interface_distribution.json");
        let method_freq = if method_freq_path.exists() {
            method_freq::MethodFreqMap::from_file_path(&method_freq_path).ok()
        } else {
            None
        };

        Ok(TraceConfig {
            call_graph,
            method_freq_map: method_freq,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::svc::ServiceName;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_read_config() {
        let temp = TempDir::new().expect("create temp dir");
        let dir = temp.path();

        let edges_path = dir.join("edges.csv");
        let mut edges_file = fs::File::create(&edges_path).expect("create edges");
        writeln!(edges_file, "caller,callee,weight").unwrap();
        writeln!(edges_file, "svc_alpha,svc_beta,1").unwrap();

        fs::write(
            dir.join("interface_distribution.json"),
            r#"{
  "graph_main": {
    "svc_alpha": {
      "method_a": 3,
      "method_b": 2
    }
  }
}"#,
        )
        .expect("write interface distribution");

        let config =
            TraceConfig::from_config_dir(&dir.to_path_buf()).expect("Reading should not fail");

        let svc_name = ServiceName::from_string("svc_alpha".into());

        assert!(config.call_graph.callees_of(&svc_name).len() > 0);

        if let Some(method_freq) = config.method_freq_map {
            let mut rng = rand::rng();
            let sampled_method = method_freq
                .sample_method(&svc_name, "graph_main", &mut rng)
                .expect("service must exist");
            assert!(["method_a", "method_b"].contains(&sampled_method.method.as_str()));
        }
    }
}

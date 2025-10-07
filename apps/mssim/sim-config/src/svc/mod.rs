use std::{borrow::Cow, fmt::Display, path::PathBuf};

pub mod call_graph;
pub mod method_freq;
pub mod method_latency;

use method_latency::MethodLatencyDistMap;
use serde::{Deserialize, Serialize};

pub type MethodId = Cow<'static, str>;

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

pub struct ServiceTraceConfig {
    pub method_latency: Option<MethodLatencyDistMap>,
    pub method_freq_map: Option<method_freq::MethodFreqMap>,
    pub call_graph: call_graph::CallGraph,
}

impl ServiceTraceConfig {
    pub fn from_config_dir(
        dir: &PathBuf,
        svc_name: Option<ServiceName>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let call_graph_path = dir.join("edges.csv");
        let call_graph = call_graph::CallGraph::from_path(call_graph_path)?;

        let method_latency = match svc_name {
            Some(svc_name) => {
                let method_latency_path = dir.join("latency_percentiles.json");
                if method_latency_path.exists() {
                    let map = MethodLatencyDistMap::from_file_path(&method_latency_path, svc_name)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    Some(map)
                } else {
                    None
                }
            }
            None => None,
        };

        let method_freq_path = dir.join("interface_distribution.json");
        if method_freq_path.exists() {
            let method_freq = method_freq::MethodFreqMap::from_file_path(&method_freq_path)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

            return Ok(ServiceTraceConfig {
                call_graph,
                method_latency,
                method_freq_map: Some(method_freq),
            });
        } else {
            return Ok(ServiceTraceConfig {
                call_graph,
                method_latency,
                method_freq_map: None,
            });
        }
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
        let svc_name = ServiceName::from_string("MS_49817".into());
        let path = workspace_root().join("./trace-analysis/golden/S_32048416/");

        let config = ServiceTraceConfig::from_config_dir(&path, Some(svc_name.clone()))
            .expect("Reading should not fail");

        assert!(config.call_graph.callees_of(&svc_name).len() > 0);

        let method = "daq6sEhEBy".into();
        let dist = config
            .method_latency
            .as_ref()
            .expect("Method latency should be present")
            .get_method_dist(&method)
            .expect("Method distribution should exist");

        let p50 = dist.sample(&mut rand::rng());
        assert!(p50 > 0.0);

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

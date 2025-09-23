use crate::svc::{MethodId, ServiceName};
use anyhow::{Context, Result};
use std::{collections::HashMap, fs, path::PathBuf};

use crate::dist::Distribution;

/// Stores the distribution configuration for this service
#[derive(Debug)]
pub struct MethodLatencyDistMap {
    methods: HashMap<MethodId, Distribution>,
}

/// Raw deserialization shape matching the file:
/// HashMap<microservice, HashMap<method, HashMap<percentile_string, latency_f64>>>
type RawFileShape = HashMap<ServiceName, HashMap<String, HashMap<String, f64>>>;

// parsing logic
impl MethodLatencyDistMap {
    pub fn from_file_path(path: &PathBuf, service_name: ServiceName) -> Result<Self> {
        let config_str = fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        Self::from_str(&config_str, service_name)
    }

    fn from_str(config_str: &str, service_name: ServiceName) -> Result<Self> {
        let raw: RawFileShape = serde_json::from_str(config_str).context("Invalid JSON")?;
        Self::parse_distributions(raw, service_name)
    }

    fn parse_distributions(mut raw: RawFileShape, service_name: ServiceName) -> Result<Self> {
        let our_svc = raw
            .remove(&service_name)
            .with_context(|| format!("Service name {} doesn't exist in config", service_name))?;

        let mut methods = HashMap::new();
        for (method, p2l) in our_svc {
            let dist = Distribution::from_percentile_map(&p2l)
                .with_context(|| format!("While parsing {service_name}.{method}"))?;
            methods.insert(method.into(), dist);
        }

        Ok(MethodLatencyDistMap { methods })
    }
}

impl MethodLatencyDistMap {
    pub fn get_method_dist(&self, method: &MethodId) -> Option<&Distribution> {
        self.methods.get(method)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        env!("CARGO_WORKSPACE_DIR").into()
    }

    #[test]
    fn test_parsing() {
        let svc_name = ServiceName::new("MS_11603");
        let path =
            workspace_root().join("./trace-analysis/golden/S_32048416/latency_percentiles.json");

        let dist_map =
            MethodLatencyDistMap::from_file_path(&path, svc_name).expect("Parsing should not fail");

        let method = "47lZCv__NT:TDDL_QUERY".into();
        let dist = dist_map
            .get_method_dist(&method)
            .expect("Method distribution should exist");

        // The min and max are manually derived from the distribution in the golden file.
        let sample_min = 12.0;
        let sample_max = 13.0;
        for i in 0..99 {
            let sample = dist.quantile(i.into());
            assert!(sample >= sample_min);
            assert!(sample <= sample_max);
        }

        let percentiles = [99.1, 99.5, 99.9, 99.99];
        for p in percentiles {
            let sample = dist.quantile(p);
            assert!(sample >= sample_min);
            assert!(sample <= sample_max);
        }
    }
}

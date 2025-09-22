//! Parsing simulation configuration.
//!
//! (Note: this is a new configuration schema that is different from what's parsed in main.rs now)

#![allow(dead_code)]

use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{Context, Result};

use dist::Distribution;

mod dist;

/// Raw deserialization shape matching the file:
/// HashMap<microservice, HashMap<method, HashMap<percentile_string, latency_f64>>>
type RawFileShape = HashMap<String, HashMap<String, HashMap<String, f64>>>;

/// Final shape:
/// HashMap<microservice, HashMap<method, Distribution>>
type DistMap = HashMap<String, HashMap<String, Distribution>>;

fn parse_distributions(raw: RawFileShape) -> Result<DistMap> {
    let mut out: DistMap = HashMap::new();
    for (svc, methods) in raw {
        let mut method_map: HashMap<String, Distribution> = HashMap::new();
        for (method, p2l) in methods {
            let dist = Distribution::from_percentile_map(&p2l)
                .with_context(|| format!("While parsing {svc}.{method}"))?;
            method_map.insert(method, dist);
        }
        out.insert(svc, method_map);
    }
    Ok(out)
}

fn load_from_str(s: &str) -> Result<DistMap> {
    let raw: RawFileShape = serde_json::from_str(s).context("Invalid JSON")?;
    parse_distributions(raw)
}

fn load_from_file(path: &PathBuf) -> Result<DistMap> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    load_from_str(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        env!("CARGO_WORKSPACE_DIR").into()
    }

    #[test]
    fn test_parsing() {
        let path =
            workspace_root().join("./trace-analysis/golden/S_32048416/latency_percentiles.json");
        let dist_map = load_from_file(&path).expect("Parsing should not fail");

        let svc_name = "MS_11603";

        let svc_map = dist_map.get(svc_name).expect("Service should exist");

        let method = "47lZCv__NT:TDDL_QUERY";
        let dist = svc_map
            .get(method)
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

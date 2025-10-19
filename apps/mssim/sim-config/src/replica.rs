use std::{collections::HashMap, path::PathBuf};

use crate::svc::ServiceName;
use serde::Deserialize;

const DEFAULT_REPLICA_COUNT: u32 = 1;

#[derive(Debug)]
pub struct ReplicaConfig {
    default: u32,
    overrides: HashMap<ServiceName, u32>,
}

impl Default for ReplicaConfig {
    fn default() -> Self {
        Self {
            default: DEFAULT_REPLICA_COUNT,
            overrides: HashMap::new(),
        }
    }
}

#[derive(Deserialize)]
struct RawReplicaConfig {
    #[serde(default = "default_replica_count")]
    default: u32,
    #[serde(default)]
    overrides: HashMap<ServiceName, u32>,
}

fn default_replica_count() -> u32 {
    DEFAULT_REPLICA_COUNT
}

impl ReplicaConfig {
    pub fn read_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)?;
        let raw_cfg: RawReplicaConfig = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self {
            default: raw_cfg.default,
            overrides: raw_cfg.overrides,
        })
    }

    pub fn get(&self, service: &ServiceName) -> Option<u32> {
        self.overrides.get(service).copied()
    }

    pub fn count_for(&self, service: &ServiceName) -> u32 {
        self.get(service).unwrap_or(self.default)
    }

    pub fn default_replica_count(&self) -> u32 {
        self.default
    }

    pub fn is_empty(&self) -> bool {
        self.overrides.is_empty()
    }
}

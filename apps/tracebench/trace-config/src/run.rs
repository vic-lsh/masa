//! Config for running a microservice trace replay (e.g., # replicas, load generation).

use anyhow::Result;
use std::path::PathBuf;

use crate::replica::ReplicaConfig;

#[derive(Default)]
pub struct ReplayConfig {
    pub replicas: ReplicaConfig,
}

impl ReplayConfig {
    pub fn from_config_dir(config_dir: &PathBuf) -> Result<Self> {
        let path = config_dir.join("replicas.json");
        let replicas = ReplicaConfig::read_from_file(&path)
            .map_err(|e| anyhow::anyhow!("Failed to parse config directory: {}", e))?;
        Ok(ReplayConfig { replicas })
    }
}

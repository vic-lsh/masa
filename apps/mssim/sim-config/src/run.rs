//! Config for running a microservice simulation (e.g., # replicas, load generation).

use anyhow::Result;
use std::path::PathBuf;

use crate::replica::ReplicaConfig;

#[derive(Default)]
pub struct SimulatorConfig {
    pub replicas: ReplicaConfig,
}

impl SimulatorConfig {
    pub fn from_config_dir(config_dir: &PathBuf) -> Result<Self> {
        let path = config_dir.join("replicas.json");
        let replicas = ReplicaConfig::read_from_file(&path)
            .map_err(|e| anyhow::anyhow!("Failed to parse config directory: {}", e))?;
        Ok(SimulatorConfig { replicas })
    }
}

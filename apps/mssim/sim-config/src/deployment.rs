//! Service discovery info for a simulator deployment

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

// TODO: support replicas
#[derive(Debug, Deserialize, Serialize)]
pub struct ServiceDiscoveryInfo {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Deployment {
    pub services: HashMap<String, ServiceDiscoveryInfo>,
}

impl Deployment {
    pub fn read_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let deployment: Deployment = serde_json::from_str(&content)?;
        Ok(deployment)
    }

    pub fn export_to_file(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn add_service(&mut self, name: String, info: ServiceDiscoveryInfo) {
        self.services.insert(name, info);
    }
}

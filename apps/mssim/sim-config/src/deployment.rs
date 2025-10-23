//! Service discovery info for a simulator deployment

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::svc::ServiceName;

// TODO: support replicas
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceDiscoveryInfo {
    pub ip: String,
    pub port: u16,
    pub replicas: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Deployment {
    pub services: HashMap<ServiceName, ServiceDiscoveryInfo>,
}

impl Deployment {
    pub fn new() -> Self {
        Deployment {
            services: HashMap::new(),
        }
    }

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

    pub fn add_service(&mut self, name: ServiceName, info: ServiceDiscoveryInfo) {
        self.services.insert(name, info);
    }
}

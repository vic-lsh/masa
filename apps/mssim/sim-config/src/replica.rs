use std::{collections::HashMap, path::PathBuf};

use crate::svc::ServiceName;

#[derive(Default)]
pub struct ReplicaConfig {
    replica_counts: HashMap<ServiceName, u32>,
}

impl ReplicaConfig {
    pub fn read_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)?;
        let replica_counts: HashMap<ServiceName, u32> = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Self { replica_counts })
    }

    pub fn get(&self, service: &ServiceName) -> Option<u32> {
        self.replica_counts.get(service).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.replica_counts.is_empty()
    }
}

use serde::{Deserialize, Serialize};
use serde_json;

/// Represent Masa Information.
#[derive(Serialize, Deserialize, Debug)]
pub struct Masainfo {
    start_at: u64,
    deadline: u64,
}

impl Masainfo {
    /// Create a new Masainfo.
    pub fn new(start_at: u64, deadline: u64) -> Self {
        Self { start_at, deadline }
    }

    /// Create a new Masainfo from JSON.
    pub fn from_json(json: &str) -> Self {
        serde_json::from_str(json).unwrap()
    }

    /// Convert Masainfo to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }
}

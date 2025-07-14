use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use super::SimulatorConfig;

/// Parse a JSON file into a SimulatorConfig
pub fn parse_json_file(path: &Path) -> Result<SimulatorConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read JSON file: {}", path.display()))?;

    parse_json_str(&content)
}

/// Parse a JSON string into a SimulatorConfig
pub fn parse_json_str(content: &str) -> Result<SimulatorConfig> {
    let config: SimulatorConfig =
        serde_json::from_str(content).context("Failed to parse JSON content")?;

    Ok(config)
}

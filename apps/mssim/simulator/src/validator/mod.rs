pub mod rules;

use crate::parser::SimulatorConfig;
use anyhow::Result;

/// Validate a simulator configuration
pub fn validate_config(config: &SimulatorConfig) -> Result<()> {
    // Run all validation rules
    rules::validate_has_services(config)?;
    rules::validate_service_dependencies(config)?;
    rules::validate_latency_distributions(config)?;
    rules::validate_error_rates(config)?;

    // If load configuration is present, validate it
    if let Some(load) = &config.load {
        rules::validate_load_config(load, config)?;
    }

    Ok(())
}

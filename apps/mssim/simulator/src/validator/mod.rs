pub mod rules;

use anyhow::Result;
use sim_config::trace::TraceConfig;

/// Validate a simulator configuration
pub fn validate_config(config: &TraceConfig) -> Result<()> {
    // Run all validation rules
    rules::validate_has_services(config)?;
    rules::validate_service_dependencies(config)?;
    Ok(())
}

// Configuration validation logic

use std::collections::HashMap;

use super::types::*;
use crate::constants::SERVICE_METHOD_SEPARATOR;
use crate::error::{ConfigError, Result, SyntheticError};
use crate::types::ServiceId;

/// Parse a "service_name::method_name" string into a CallTarget
pub fn parse_service_method(s: &str) -> Result<CallTarget> {
    let parts: Vec<&str> = s.split(SERVICE_METHOD_SEPARATOR).collect();
    if parts.len() != 2 {
        return Err(SyntheticError::Parse(format!(
            "Invalid service::method format: '{}'. Expected 'service_name{}method_name'",
            s, SERVICE_METHOD_SEPARATOR
        )));
    }

    let service_id = ServiceId::new(parts[0])?;
    let method_name = crate::types::MethodName::new(parts[1])?;

    Ok(CallTarget {
        service_id: service_id.into_string(),
        method_name: method_name.into_string(),
    })
}

/// Validate that all referenced services and methods exist in the call graph
pub fn validate_call_graph(config: &CallGraphConfig) -> Result<()> {
    // Build a set of all valid service::method combinations
    let mut valid_targets = HashMap::new();
    for service in &config.services {
        for method in &service.methods {
            let target = format!("{}{}{}", service.id, SERVICE_METHOD_SEPARATOR, method.name);
            valid_targets.insert(target, (service.id.clone(), method.name.clone()));
        }
    }

    // Validate entry_point
    if !valid_targets.contains_key(&config.entry_point) {
        return Err(SyntheticError::EntryPointNotFound(format!(
            "Entry point '{}' does not exist in call graph",
            config.entry_point
        )));
    }

    // Validate all call sequences
    for service in &config.services {
        for method in &service.methods {
            for step in &method.call_sequence_raw {
                for (target_str, _prob) in step {
                    if !valid_targets.contains_key(target_str) {
                        return Err(SyntheticError::Validation(format!(
                            "Service '{}' method '{}' references non-existent target '{}'",
                            service.id, method.name, target_str
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Validate service configuration
pub fn validate_service_config(service: &ServiceDefinition) -> Result<()> {
    if service.id.trim().is_empty() {
        return Err(
            ConfigError::InvalidServiceConfig("Service ID cannot be empty".to_string()).into(),
        );
    }

    if service.replicas == 0 {
        return Err(ConfigError::InvalidServiceConfig(
            "Service must have at least 1 replica".to_string(),
        )
        .into());
    }

    if service.methods.is_empty() {
        return Err(ConfigError::InvalidServiceConfig(
            "Service must have at least one method".to_string(),
        )
        .into());
    }

    // Validate methods
    let mut method_names = std::collections::HashSet::new();
    for method in &service.methods {
        if method.name.trim().is_empty() {
            return Err(ConfigError::InvalidServiceConfig(
                "Method name cannot be empty".to_string(),
            )
            .into());
        }

        if !method_names.insert(&method.name) {
            return Err(ConfigError::InvalidServiceConfig(format!(
                "Duplicate method name '{}' in service '{}'",
                method.name, service.id
            ))
            .into());
        }

        // Validate busy spin ratio
        if let Some(ratio) = method.busy_spin_ratio {
            if ratio < 0.0 || ratio > 1.0 {
                return Err(ConfigError::InvalidServiceConfig(format!(
                    "Invalid busy_spin_ratio {} for method '{}'. Must be between 0.0 and 1.0",
                    ratio, method.name
                ))
                .into());
            }
        }

        // Validate latency distribution
        method.latency_distribution.validate()?;
    }

    Ok(())
}

/// Validate complete synthetic configuration
pub fn validate_synthetic_config(config: &SyntheticConfig) -> Result<()> {
    // Validate child services if present
    for service in &config.child_services {
        if service.id.trim().is_empty() {
            return Err(ConfigError::InvalidServiceConfig(
                "Child service ID cannot be empty".to_string(),
            )
            .into());
        }
    }

    // Validate request hops if present
    validate_request_hops(&config.request_a_hops, "request_a_hops")?;
    validate_request_hops(&config.request_b_hops, "request_b_hops")?;

    // Validate call graph if present
    if let Some(ref call_graph) = config.call_graph {
        validate_call_graph(call_graph)?;

        // Validate each service in the call graph
        for service in &call_graph.services {
            validate_service_config(service)?;
        }
    }

    // Validate random latency distribution
    config.child_random_latency.validate()?;

    Ok(())
}

/// Validate request hops configuration
fn validate_request_hops(hops: &[RequestHop], field_name: &str) -> Result<()> {
    for hop in hops {
        if hop.service_id.trim().is_empty() {
            return Err(ConfigError::InvalidServiceConfig(format!(
                "Service ID in {} cannot be empty",
                field_name
            ))
            .into());
        }

        if let Some(duration) = hop.duration_us {
            if duration == 0 {
                return Err(ConfigError::InvalidServiceConfig(format!(
                    "Duration in {} must be greater than 0",
                    field_name
                ))
                .into());
            }
        }

        if let Some(busy_spin) = hop.busy_spin_dur_us {
            if let Some(duration) = hop.duration_us {
                if busy_spin > duration {
                    return Err(ConfigError::InvalidServiceConfig(format!(
                        "Busy spin duration cannot exceed total duration in {}",
                        field_name
                    ))
                    .into());
                }
            }
        }
    }
    Ok(())
}

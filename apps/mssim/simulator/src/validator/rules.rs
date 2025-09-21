use anyhow::{Result, bail};
use std::collections::HashSet;

use crate::parser::{Distribution, EntryPoint, LoadConfig, SimulatorConfig};

/// Validate that the configuration has at least one service
pub fn validate_has_services(config: &SimulatorConfig) -> Result<()> {
    if config.services.is_empty() {
        bail!("Configuration must define at least one service");
    }
    Ok(())
}

/// Validate that all service dependencies exist
pub fn validate_service_dependencies(config: &SimulatorConfig) -> Result<()> {
    let service_names: HashSet<&String> = config.services.keys().collect();

    // Check that all referenced services exist
    for (service_name, service) in &config.services {
        for (method_name, method) in &service.methods {
            for call_sequence in &method.calls {
                for call in call_sequence {
                    let parts: Vec<&str> = call.split('.').collect();
                    if parts.len() != 2 {
                        bail!(
                            "Invalid call format in {}.{}: '{}'. Expected 'ServiceName.MethodName'",
                            service_name,
                            method_name,
                            call
                        );
                    }

                    let called_service = parts[0];
                    let called_method = parts[1];

                    // Check if called service exists
                    if !service_names.contains(&called_service.to_string()) {
                        bail!(
                            "Service '{}' called by {}.{} does not exist",
                            called_service,
                            service_name,
                            method_name
                        );
                    }

                    // Check if called method exists in that service
                    if !config.services[called_service]
                        .methods
                        .contains_key(called_method)
                    {
                        bail!(
                            "Method '{}' called on service '{}' does not exist",
                            called_method,
                            called_service
                        );
                    }
                }
            }
        }
    }

    // Check for circular dependencies using a simple DFS algorithm
    detect_circular_dependencies(config)?;

    Ok(())
}

/// Validate that all latency distributions are valid
pub fn validate_latency_distributions(config: &SimulatorConfig) -> Result<()> {
    for (service_name, service) in &config.services {
        for (method_name, method) in &service.methods {
            validate_single_distribution(&method.latency_distribution, service_name, method_name)?;
        }
    }
    Ok(())
}

/// Validate a single distribution
fn validate_single_distribution(
    distribution: &Distribution,
    service_name: &str,
    method_name: &str,
) -> Result<()> {
    match distribution.distribution_type.as_str() {
        "normal" => {
            // Check required parameters for Normal distribution
            if !distribution.parameters.contains_key("mean") {
                bail!(
                    "Normal distribution for {}.{} missing 'mean' parameter",
                    service_name,
                    method_name
                );
            }
            if !distribution.parameters.contains_key("stddev") {
                bail!(
                    "Normal distribution for {}.{} missing 'stddev' parameter",
                    service_name,
                    method_name
                );
            }

            // Validate mean is non-negative
            if distribution.parameters["mean"] < 0.0 {
                bail!(
                    "Normal distribution for {}.{} has negative mean: {}",
                    service_name,
                    method_name,
                    distribution.parameters["mean"]
                );
            }

            // Validate stddev is positive
            if distribution.parameters["stddev"] <= 0.0 {
                bail!(
                    "Normal distribution for {}.{} has non-positive stddev: {}",
                    service_name,
                    method_name,
                    distribution.parameters["stddev"]
                );
            }
        }
        "uniform" => {
            // Check required parameters for Uniform distribution
            if !distribution.parameters.contains_key("min") {
                bail!(
                    "Uniform distribution for {}.{} missing 'min' parameter",
                    service_name,
                    method_name
                );
            }
            if !distribution.parameters.contains_key("max") {
                bail!(
                    "Uniform distribution for {}.{} missing 'max' parameter",
                    service_name,
                    method_name
                );
            }

            // Validate min <= max
            if distribution.parameters["min"] > distribution.parameters["max"] {
                bail!(
                    "Uniform distribution for {}.{} has min > max: {} > {}",
                    service_name,
                    method_name,
                    distribution.parameters["min"],
                    distribution.parameters["max"]
                );
            }

            // Validate min is non-negative
            if distribution.parameters["min"] < 0.0 {
                bail!(
                    "Uniform distribution for {}.{} has negative min: {}",
                    service_name,
                    method_name,
                    distribution.parameters["min"]
                );
            }
        }
        "constant" => {
            // Check required parameter for Constant distribution
            if !distribution.parameters.contains_key("value") {
                bail!(
                    "Constant distribution for {}.{} missing 'value' parameter",
                    service_name,
                    method_name
                );
            }

            // Validate value is non-negative
            if distribution.parameters["value"] < 0.0 {
                bail!(
                    "Constant distribution for {}.{} has negative value: {}",
                    service_name,
                    method_name,
                    distribution.parameters["value"]
                );
            }
        }
        "exponential" => {
            // Check required parameter for Exponential distribution
            if !distribution.parameters.contains_key("rate") {
                bail!(
                    "Exponential distribution for {}.{} missing 'rate' parameter",
                    service_name,
                    method_name
                );
            }

            // Validate rate is positive
            if distribution.parameters["rate"] <= 0.0 {
                bail!(
                    "Exponential distribution for {}.{} has non-positive rate: {}",
                    service_name,
                    method_name,
                    distribution.parameters["rate"]
                );
            }
        }
        "bernoulli" => {
            if !distribution.parameters.contains_key("p") {
                bail!(
                    "Exponential distribution for {}.{} missing 'p' parameter",
                    service_name,
                    method_name
                )
            }
        }
        _ => {
            bail!(
                "Unknown distribution type for {}.{}: '{}'",
                service_name,
                method_name,
                distribution.distribution_type
            );
        }
    }
    Ok(())
}

/// Detect circular dependencies in the service call graph
fn detect_circular_dependencies(config: &SimulatorConfig) -> Result<()> {
    // Track visited services in current call stack
    let mut visited = HashSet::new();
    let mut stack = HashSet::new();

    // Run DFS on each service
    for service_name in config.services.keys() {
        if !visited.contains(service_name) {
            if detect_cycles_dfs(config, service_name, &mut visited, &mut stack)? {
                return Ok(());
            }
        }
    }

    Ok(())
}

/// Helper function for DFS cycle detection
fn detect_cycles_dfs(
    config: &SimulatorConfig,
    service_name: &String,
    visited: &mut HashSet<String>,
    stack: &mut HashSet<String>,
) -> Result<bool> {
    visited.insert(service_name.clone());
    stack.insert(service_name.clone());

    // Check all methods in this service
    let service = &config.services[service_name];
    for method in service.methods.values() {
        for call_sequence in &method.calls {
            for call in call_sequence {
                let parts: Vec<&str> = call.split('.').collect();
                let called_service = parts[0].to_string();

                // If this called service is already in our call stack, we have a cycle
                if stack.contains(&called_service) {
                    bail!(
                        "Circular dependency detected: Service '{}' depends on '{}'",
                        service_name,
                        called_service
                    );
                }

                // If we haven't visited this called service yet, recursively check it
                if !visited.contains(&called_service) {
                    if detect_cycles_dfs(config, &called_service, visited, stack)? {
                        return Ok(true);
                    }
                }
            }
        }
    }

    // Remove from current path stack when we're done exploring this service
    stack.remove(service_name);
    Ok(false)
}

/// Validate error rates for all methods in all services
pub fn validate_error_rates(config: &SimulatorConfig) -> Result<()> {
    for (service_name, service) in &config.services {
        for (method_name, method) in &service.methods {
            // Check if error_rate exists
            if let Some(error_rate) = &method.error_rate {
                validate_single_distribution(error_rate, service_name, method_name)?;
            }
        }
    }
    Ok(())
}

/// Validate load configuration
pub fn validate_load_config(load: &LoadConfig, config: &SimulatorConfig) -> Result<()> {
    // Ensure there's at least one entry point
    if load.entry_points.is_empty() {
        bail!("Load configuration must have at least one entry point");
    }

    // Validate each entry point
    for (index, entry_point) in load.entry_points.iter().enumerate() {
        validate_entry_point(entry_point, config, index)?;
    }

    Ok(())
}

/// Validate a single entry point
fn validate_entry_point(
    entry_point: &EntryPoint,
    config: &SimulatorConfig,
    index: usize,
) -> Result<()> {
    // Validate service exists
    if !config.services.contains_key(&entry_point.service) {
        bail!(
            "Entry point service '{}' at index {} does not exist",
            entry_point.service,
            index
        );
    }

    // Validate method exists in that service
    if !config.services[&entry_point.service]
        .methods
        .contains_key(&entry_point.method)
    {
        bail!(
            "Entry point method '{}' at index {} does not exist in service '{}'",
            entry_point.method,
            index,
            entry_point.service
        );
    }

    // Validate requests per second - must be positive (u32 is already non-negative)
    if entry_point.requests_per_second == 0 {
        bail!(
            "Entry point requests_per_second at index {} must be positive",
            index
        );
    }

    Ok(())
}

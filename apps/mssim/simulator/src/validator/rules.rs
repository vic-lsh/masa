use anyhow::{bail, Result};
use sim_config::{svc::ServiceName, trace::TraceConfig};
use std::collections::HashSet;

use crate::parser::SimulatorConfig;

/// Validate that the configuration has at least one service
pub fn validate_has_services(config: &TraceConfig) -> Result<()> {
    if config.call_graph.services().is_empty() {
        bail!("Configuration must define at least one service");
    }
    Ok(())
}

pub fn validate_service_dependencies(config: &TraceConfig) -> Result<()> {
    // TODO: relax the DAG constraint somewhat.
    detect_circular_dependencies(config)
}

/// Detect circular dependencies in the service call graph
fn detect_circular_dependencies(config: &TraceConfig) -> Result<()> {
    // Track visited services in current call stack
    let mut visited = HashSet::new();
    let mut stack = HashSet::new();

    let services = config.call_graph.services();

    // Run DFS on each service
    for service_name in services {
        if !visited.contains(&service_name) {
            if detect_cycles_dfs(config, &service_name, &mut visited, &mut stack)? {
                return Ok(());
            }
        }
    }

    Ok(())
}

/// Helper function for DFS cycle detection
fn detect_cycles_dfs(
    config: &TraceConfig,
    service_name: &ServiceName,
    visited: &mut HashSet<ServiceName>,
    stack: &mut HashSet<ServiceName>,
) -> Result<bool> {
    visited.insert(service_name.clone());
    stack.insert(service_name.clone());

    // Check all methods in this service
    let callees = config.call_graph.callees_of(service_name);

    for called_service in callees {
        // If this called service is already in our call stack, we have a cycle
        if stack.contains(&called_service) {
            bail!(
                "Circular dependency detected: Service '{}' and '{}' depend on each other",
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

    // for method in service.methods.values() {
    //     for call_sequence in &method.calls {
    //         for call in call_sequence {
    //             let parts: Vec<&str> = call.split('.').collect();
    //             let called_service = parts[0].to_string();

    //             // If this called service is already in our call stack, we have a cycle
    //             if stack.contains(&called_service) {
    //                 bail!(
    //                     "Circular dependency detected: Service '{}' and '{}' depend on each other",
    //                     service_name,
    //                     called_service
    //                 );
    //             }

    //             // If we haven't visited this called service yet, recursively check it
    //             if !visited.contains(&called_service) {
    //                 if detect_cycles_dfs(config, &called_service, visited, stack)? {
    //                     return Ok(true);
    //                 }
    //             }
    //         }
    //     }
    // }

    // Remove from current path stack when we're done exploring this service
    stack.remove(service_name);
    Ok(false)
}

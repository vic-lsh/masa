// Docker hostname resolution utilities

use crate::constants::*;

/// Resolve Docker Compose hostnames for services
pub fn resolve_hostnames(
    service_id: &str,
    _replicas: u8,
) -> Result<String, crate::error::ConnectionError> {
    // Read project name from environment variable (set by exp.runner)
    let project_name = std::env::var(DOCKER_COMPOSE_PROJECT_NAME_ENV)
        .ok()
        .filter(|s| !s.is_empty());

    // Service name matches to compose file service name: "local-{service-id}-service"
    // Docker Compose creates containers like: {project}-local-{service-id}-service-1, -2, etc.
    let base_service_name = format!("local-{}-service", service_id.to_lowercase());
    let hostname_base = if let Some(ref project) = project_name {
        format!("{}-{}", project, base_service_name)
    } else {
        base_service_name
    };

    Ok(hostname_base)
}

/// Resolve child service hostnames for hard_code mode
pub fn resolve_child_hostname(service_index: u32, replica_start: u32) -> String {
    format!(
        "{}-{}",
        DEFAULT_CHILD_HOSTNAME_BASE,
        service_index + replica_start
    )
}

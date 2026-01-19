// Connection management module for synthetic application

pub mod bootstrap;
pub mod docker;
pub mod manager;
pub mod registry;

// Re-export public API
pub use bootstrap::ConnectionBootstrap;
pub use docker::resolve_hostnames;
pub use manager::ConnectionManager;
pub use registry::ServiceRegistry;

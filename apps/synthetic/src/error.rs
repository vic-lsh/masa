// Error types for synthetic application

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyntheticError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Connection error: {0}")]
    Connection(#[from] ConnectionError),

    #[error("Invalid service ID: {0}")]
    InvalidServiceId(String),

    #[error("Invalid method name: {0}")]
    InvalidMethodName(String),

    #[error("Invalid probability: {0}")]
    InvalidProbability(String),

    #[error("Invalid duration: {0}")]
    InvalidDuration(String),

    #[error("Latency sampling error: {0}")]
    LatencySampling(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    #[error("Method not found: {0} in service {1}")]
    MethodNotFound(String, String),

    #[error("Entry point not found: {0}")]
    EntryPointNotFound(String),

    #[error("Tonic transport error: {0}")]
    TonicTransport(#[from] tonic::transport::Error),

    #[error("Tonic status error: {0}")]
    TonicStatus(#[from] tonic::Status),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Invalid latency distribution: {0}")]
    InvalidLatencyDistribution(String),

    #[error("Invalid call graph: {0}")]
    InvalidCallGraph(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid service configuration: {0}")]
    InvalidServiceConfig(String),

    #[error("Invalid exponential distribution: {0}")]
    InvalidExponential(String),

    #[error("Invalid normal distribution: {0}")]
    InvalidNormal(String),

    #[error("Invalid discrete distribution: {0}")]
    InvalidDiscrete(String),

    #[error("Invalid periodic distribution: {0}")]
    InvalidPeriodic(String),
}

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("Failed to connect to service {0}: {1}")]
    FailedToConnect(String, String),

    #[error("Service not available: {0}")]
    ServiceNotAvailable(String),

    #[error("Docker hostname resolution failed: {0}")]
    HostnameResolution(String),

    #[error("Bootstrap task failed: {0}")]
    BootstrapFailed(String),

    #[error("Registry error: {0}")]
    RegistryError(String),

    #[error("Duplicate service ID: {0}")]
    DuplicateService(String),
}

pub type Result<T> = std::result::Result<T, SyntheticError>;

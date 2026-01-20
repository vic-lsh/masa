// Constants for the synthetic application

/// Default port for frontend service
pub const DEFAULT_FRONTEND_PORT: u16 = 8660;

/// Default port range start for child services  
pub const DEFAULT_CHILD_PORT_START: u16 = 8661;

/// Default IP address for frontend
pub const DEFAULT_FRONTEND_IP: &str = "127.0.0.1";

/// Default hostname base for child services
pub const DEFAULT_CHILD_HOSTNAME_BASE: &str = "local-child-service";

/// Default hostname pattern for call graph services
pub const DEFAULT_CALL_GRAPH_SERVICE_PATTERN: &str = "local-{service-id}-service";

/// Docker Compose project name environment variable
pub const DOCKER_COMPOSE_PROJECT_NAME_ENV: &str = "DOCKER_COMPOSE_PROJECT_NAME";

/// Service ID environment variable for call graph mode
pub const SERVICE_ID_ENV: &str = "SERVICE_ID";

/// Default latency in microseconds for exponential distribution
pub const DEFAULT_EXPONENTIAL_LATENCY_US: u64 = 10000;

/// Default busy spin interval in microseconds
pub const DEFAULT_BUSY_SPIN_INTERVAL_US: u64 = 200;

/// Queue monitoring interval in milliseconds
pub const QUEUE_MONITORING_INTERVAL_MS: u64 = 500;

/// Default child CPU per replica
pub const DEFAULT_CHILD_CPU_PER_REPLICA: f64 = 1.0;

/// Default number of replicas for services
pub const DEFAULT_REPLICAS: u8 = 1;

/// Default busy spin ratio for methods (10%)
pub const DEFAULT_BUSY_SPIN_RATIO: f64 = 0.1;

/// Default probability for making calls
pub const DEFAULT_CALL_PROBABILITY: f64 = 1.0;

/// Minimum positive value for durations and probabilities
pub const MIN_POSITIVE_VALUE: f64 = 0.000001;

/// Maximum reasonable latency in microseconds (1 second)
pub const MAX_LATENCY_US: u64 = 1_000_000;

/// Service name separator for parsing "service::method"
pub const SERVICE_METHOD_SEPARATOR: &str = "::";

/// Docker Compose replica numbering start
pub const DOCKER_REPLICA_START: u32 = 1;

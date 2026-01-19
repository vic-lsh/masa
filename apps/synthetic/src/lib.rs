pub mod bootstrap;
pub mod child;
pub mod client;
pub mod config;
pub mod constants;
pub mod distribution;
pub mod error;
pub mod frontend;
pub mod service_registry;
pub mod tests;
pub mod types;
pub mod util;

pub mod tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }

    pub mod child {
        tonic::include_proto!("child");
    }
}

// Re-export for binary access
pub use child::server::ChildImpl;
pub use config::SyntheticConfig;
pub use distribution::LatencyDistribution;
pub use frontend::server::FrontendImpl;
pub use tonic::child::child_server::ChildServer;
pub use tonic::frontend::frontend_server::FrontendServer;

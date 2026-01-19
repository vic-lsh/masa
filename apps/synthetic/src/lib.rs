pub mod config;
pub mod service_registry;
pub mod util;
pub mod bootstrap;

pub mod tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }

    pub mod child {
        tonic::include_proto!("child");
    }
}

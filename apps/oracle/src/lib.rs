pub mod config;
pub mod util;

pub mod tonic {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }

    pub mod child {
        tonic::include_proto!("child");
    }
}

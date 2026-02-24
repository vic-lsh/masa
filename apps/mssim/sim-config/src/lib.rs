//! Parsing simulation configuration.

#![allow(dead_code)]

pub mod deployment;
pub mod dist;
pub mod replica;
pub mod run;
pub mod svc;
pub mod trace;

pub use run::SimulatorConfig;

pub const PROJECT_NAME: &str = "mssim";

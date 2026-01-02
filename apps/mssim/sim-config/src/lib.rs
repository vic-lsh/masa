//! Parsing simulation configuration.

#![allow(dead_code)]
#![feature(str_as_str)]

pub mod deployment;
pub mod dist;
pub mod replica;
pub mod run;
pub mod svc;
pub mod trace;

pub use run::SimulatorConfig;

pub const PROJECT_NAME: &str = "mssim";

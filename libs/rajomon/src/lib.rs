// Rajomon: token-based admission control with price propagation (NSDI '25,
// "Rajomon: Decentralized and Coordinated Overload Control for
// Latency-Sensitive Microservices"), written as an `rpcstack` module. It
// depends on the generic framework only: a request is known by its service
// and method name, and everything else Rajomon needs travels in its own wire
// section.

mod client;
mod module;
mod params;
mod shared;

#[cfg(test)]
mod tests;

pub use client::{ClientTokenBucket, CLIENT_TOKEN_BUCKET};
pub use module::{RajomonModule, RajomonServer, RajomonWire};
pub use params::{RajomonParams, LEGACY_PARAMS_PATH_ENV, PARAMS_PATH_ENV};
pub use shared::{RajomonSharedState, RAJOMON_STATE};

mod paper;
pub use paper::{PaperRajomonModule, PaperRajomonServer};

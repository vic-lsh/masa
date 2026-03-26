//! Core types for the tonic gRPC framework.
//!
//! This crate contains the fundamental types used across tonic: `Request`, `Response`,
//! `Status`, metadata, and the Masa hook trait definitions. It exists as a separate
//! crate to break circular dependencies between `tonic` and `masa-policy`.

#![warn(missing_debug_implementations, missing_docs, rust_2018_idioms)]
// Many items are pub for tonic to access but not re-exported at the crate root.
#![allow(unreachable_pub, dead_code)]
#![feature(trait_alias)]

pub mod body;
pub mod metadata;

/// Masa-related modules.
pub mod masa;

#[doc(hidden)]
pub mod extensions;
#[doc(hidden)]
pub mod request;
#[doc(hidden)]
pub mod response;
#[doc(hidden)]
pub mod status;
pub(crate) mod util;

pub use extensions::{CowGrpcMethod, Extensions, GrpcMethod};
pub use request::{IntoRequest, IntoStreamingRequest, Request};
pub use response::Response;
pub use status::{Code, Status};

pub use http;

/// A boxed error type used internally.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// `Result` is a type that represents either success ([`Ok`]) or failure ([`Err`]).
/// By default, the Err value is of type [`Status`] but this can be overridden if desired.
pub type Result<T, E = Status> = std::result::Result<T, E>;

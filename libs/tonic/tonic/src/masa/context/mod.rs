use std::{sync::Arc, task::Poll};

use crate::metadata::{Ascii, MetadataValue};
use crate::{body::BoxBody, CowGrpcMethod, GrpcMethod, Request, Response, Status};

mod common;
#[cfg(any(feature = "adctl", feature = "prio_local"))]
pub(crate) mod estimator;
mod fifo;
mod global;
#[cfg(any(feature = "adctl", feature = "prio_local"))]
pub(crate) mod latency_map;
#[cfg(feature = "prio_local")]
mod local;
mod noop;
mod prio_oldest;
mod queue_global;
mod rajomon;

#[cfg(feature = "adctl")]
pub(crate) mod adctl;
#[cfg(any(feature = "adctl", feature = "prio_local"))]
pub(crate) mod adctl_hooks;

#[cfg(all(feature = "adctl", not(feature = "early")))]
compile_error!("Feature 'adctl' requires 'early'");

pub mod runtime;
mod tls;
use masa_core::Context;
pub use tls::{client, server};

#[cfg(all(
    not(feature = "masa"),
    not(feature = "prio_global"),
    not(feature = "prio_oldest"),
    not(feature = "prio_local"),
    not(feature = "fifo")
))]
#[allow(missing_docs)]
pub type DefaultMasaHooks = noop::NoopMasaHooks;

#[cfg(all(
    feature = "fifo",
    not(feature = "prio_global"),
    not(feature = "prio_oldest"),
    not(feature = "prio_local")
))]
#[allow(missing_docs)]
pub type DefaultMasaHooks = fifo::Fifo;

#[cfg(any(feature = "prio_global"))]
#[allow(missing_docs)]
pub type DefaultMasaHooks = queue_global::QueueGlobal;

#[cfg(any(feature = "prio_oldest"))]
#[allow(missing_docs)]
pub type DefaultMasaHooks = prio_oldest::PrioOldest;

#[cfg(any(feature = "prio_local"))]
#[allow(missing_docs)]
pub type DefaultMasaHooks = local::local::LocalDeadlinePolicy;

// TODO: add notes on trait bounds
/// Trait for specifying the set of hooks to apply in a Masa build.
pub trait MasaHooks: Send + Sync + 'static {
    /// The server-level state and hook implementations.
    type ServerContext: ServerHooks;
    /// The state and hook implementations maintained per child RPC.
    type ChildContext: ClientHooks;
    /// The state and hook implementations maintained per in the server-side request handlers.
    type ParentContext: ParentHooks<Self::ChildContext, Self::ServerContext>;
}

/// Lifecycle hooks of a Masa server.
#[allow(unused_variables)]
pub trait ServerHooks: Send + Sync + 'static {
    /// Creates the service-level context.
    // [TODO:Vic] mark this function as async to support fetching resources
    // asynchronously. this may require async support in the tonic service constructor.
    fn new(service_name: &'static str) -> Self;
}

/// Lifecycle hooks when a client stub executes a request.
///
/// Structs that implement this trait (aliased to `ChildContext`) will be
/// constructed each time a client stub sends an RPC, and destructeed when the
/// RPC completes.
///
/// All hooks have a default, empty implementation. The only exception is `new`,
/// which is the hook constructor and must be implemented.
///
/// This struct does not need to be thread-safe -- it will not be accessed concurrently.
#[allow(unused_variables)]
pub trait ClientHooks {
    /// Construct a new ClientStubHook.
    fn new<T>(method: GrpcMethod, request: &Request<T>) -> Self;

    /// Lifecycle hook invoked just before a client stub sends an RPC.
    ///
    /// This is the last lifecycle hook to be called before the request is sent.
    /// For example, it is called _after_ hooks like `ParentHooks::before_child_rpc`.
    fn before_send<T>(&mut self, request: &mut Request<T>) {}

    /// Lifecycle hook invoked right after a client stub received a response
    /// for this RPC.
    ///
    /// This is the first lifecycle hook to be called after receiving the response.
    /// For example, it is called _before_ hooks like `ParentHooks::after_child_rpc`.
    fn after_recv<T>(&mut self, response: &mut Result<Response<T>, Status>) {}
}

/// Lifecycle hooks when the server executes a request.
///
/// Structs that implement this trait will be constructed each time a server
/// starts handling a request, and destructed when the request is complete.
///
/// All hook points have a default, empty implementation (except for `begin`,
/// which must be implemented and acts as a constructor). Implementer can choose
/// to only implement hooks they're interested in.
///
/// The implementation must be multithread-safe (i.e. `Send+Sync`). One reason
/// is that child RPCs can run in parallel, and they may invoke hook points
/// defined below from different threads.
#[allow(unused_variables)]
pub trait ParentHooks<Child, Server>: Send + Sync
where
    Child: ClientHooks,
    Server: ServerHooks,
{
    /// The first lifecycle, marking the start of a request execution.
    ///
    /// This is also the constructor for the hook point struct implementation.
    fn begin<B>(method: GrpcMethod, req: &http::Request<B>, server_ctx: Arc<Server>) -> Self;

    /// Invoked before the request handler makes an RPC.
    #[must_use]
    fn before_child_rpc<T>(
        &self,
        method: GrpcMethod,
        request: &mut Request<T>,
        child_ctx: &mut Child,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Invoked after the request handler receives a response from an RPC it made earlier.
    #[must_use]
    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        response: &mut Result<Response<T>, Status>,
        child_ctx: Child,
    ) -> Result<(), Status> {
        Ok(())
    }

    /// Invoked each time before the request handler future is polled.
    ///
    /// Being invoked indicates that the request handler can make progress.
    ///
    /// To return early without continuing request processing, return an error
    /// with the response to send back to the client.
    #[must_use]
    fn before_poll<Ret>(&self) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// Invoked each time after the request handler future is polled.
    ///
    /// The poll result shows whether the request is blocked or finalized.
    ///
    /// To return early without continuing request processing, return an error
    /// with the response to send back to the client.
    #[must_use]
    fn after_poll<Ret>(
        &self,
        poll: &Poll<Result<Response<Ret>, Status>>,
    ) -> Result<(), Result<Response<Ret>, Status>> {
        Ok(())
    }

    /// Invoked after the request handler (or the hook) has produced a response, but before it is serialized.
    ///
    /// This is useful for inspecting the response before it is serialized.
    ///
    /// `finalize_after_serialization` is invoked after this hook and after the response is serialized.
    fn finalize_before_serialization<Ret>(&self, result: &mut Result<Response<Ret>, Status>) {}

    /// The last lifecycle hook to be invoked. Provides a mutable reference to the response about
    /// to be sent back to the client.
    fn finalize_after_serialization(&self, response: &mut http::Response<BoxBody>) {}
}

/// Header key for overriding the gRPC method name in latency tracking.
///
/// When a service uses a generic gRPC method (e.g., `invoke`) to simulate multiple methods,
/// this header can be set to specify the actual method name being simulated. This allows
/// per-method latency estimates to be maintained accurately.
pub const METHOD_NAME_OVERRIDE_HEADER: &str = "x-masa-method-name";

/// Header key for overriding the gRPC service name in latency tracking.
///
/// When a service uses a generic gRPC service to simulate multiple services,
/// this header can be set to specify the actual service name being simulated.
pub const SERVICE_NAME_OVERRIDE_HEADER: &str = "x-masa-service-name";

/// Internal header key for MASA context.
pub(crate) const MASA_CONTEXT_HEADER: &str = masa_core::MASA_CONTEXT_HEADER;

#[cfg(feature = "rajomon")]
pub use rajomon::RAJOMON_STATE;

/// Get the MASA context from metadata.
pub fn get_masa_context_from_metadata(metadata: &crate::metadata::MetadataMap) -> Option<Context> {
    metadata
        .get(MASA_CONTEXT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(Context::from_header_string)
}

/// Set the MASA context in metadata.
pub fn set_masa_context_in_metadata(metadata: &mut crate::metadata::MetadataMap, ctx: &Context) {
    metadata.insert_ctx(MASA_CONTEXT_HEADER, ctx);
}

/// Extension trait for `Request<T>` to set the method name override header.
pub trait MasaRequestExt<T> {
    /// Set the method name override header on this request.
    ///
    /// This is useful when using a generic gRPC method to simulate multiple methods.
    /// The override method name will be used for latency tracking instead of the
    /// actual gRPC method name.
    ///
    /// # Arguments
    ///
    /// * `method_name` - The actual method name being simulated
    ///
    /// # Errors
    ///
    /// Returns an error if the method name cannot be converted to a valid metadata value.
    fn set_method_name_override(&mut self, method_name: &str) -> Result<(), Status>;

    /// Set the service name override header on this request.
    ///
    /// This is useful when using a generic gRPC service to simulate multiple services.
    ///
    /// # Arguments
    ///
    /// * `service_name` - The actual service name being simulated
    ///
    /// # Errors
    ///
    /// Returns an error if the service name cannot be converted to a valid metadata value.
    fn set_service_name_override(&mut self, service_name: &str) -> Result<(), Status>;

    /// Set the MASA context for this request.
    fn set_masa_context(&mut self, ctx: &Context);

    /// Set the MASA context for this request (builder style).
    fn with_masa_context(self, ctx: &Context) -> Self;

    /// Get the MASA context from this request.
    fn get_masa_context(&self) -> Option<Context>;
}

impl<T> MasaRequestExt<T> for Request<T> {
    fn set_method_name_override(&mut self, method_name: &str) -> Result<(), Status> {
        let value = MetadataValue::<Ascii>::try_from(method_name).map_err(|e| {
            Status::internal(format!(
                "Failed to create metadata value for method name override: {:?}",
                e
            ))
        })?;
        self.metadata_mut()
            .insert(METHOD_NAME_OVERRIDE_HEADER, value);
        Ok(())
    }

    fn set_service_name_override(&mut self, service_name: &str) -> Result<(), Status> {
        let value = MetadataValue::<Ascii>::try_from(service_name).map_err(|e| {
            Status::internal(format!(
                "Failed to create metadata value for service name override: {:?}",
                e
            ))
        })?;
        self.metadata_mut()
            .insert(SERVICE_NAME_OVERRIDE_HEADER, value);
        Ok(())
    }

    fn set_masa_context(&mut self, ctx: &Context) {
        set_masa_context_in_metadata(self.metadata_mut(), ctx);
    }

    fn with_masa_context(mut self, ctx: &Context) -> Self {
        set_masa_context_in_metadata(self.metadata_mut(), ctx);
        self
    }

    fn get_masa_context(&self) -> Option<Context> {
        get_masa_context_from_metadata(self.metadata())
    }
}

/// Extension trait for `Response<T>` to manage MASA context.
pub trait MasaResponseExt<T> {
    /// Set the MASA context for this response.
    fn set_masa_context(&mut self, ctx: &Context);

    /// Set the MASA context for this response (builder style).
    fn with_masa_context(self, ctx: &Context) -> Self;

    /// Get the MASA context from this response.
    fn get_masa_context(&self) -> Option<Context>;
}

impl<T> MasaResponseExt<T> for Response<T> {
    fn set_masa_context(&mut self, ctx: &Context) {
        set_masa_context_in_metadata(self.metadata_mut(), ctx);
    }

    fn with_masa_context(mut self, ctx: &Context) -> Self {
        set_masa_context_in_metadata(self.metadata_mut(), ctx);
        self
    }

    fn get_masa_context(&self) -> Option<Context> {
        get_masa_context_from_metadata(self.metadata())
    }
}

/// Extension trait for `Status` to manage MASA context.
pub trait MasaStatusExt {
    /// Set the MASA context for this status.
    fn set_masa_context(&mut self, ctx: &Context);

    /// Set the MASA context for this status (builder style).
    fn with_masa_context(self, ctx: &Context) -> Self;

    /// Get the MASA context from this status.
    fn get_masa_context(&self) -> Option<Context>;
}

impl MasaStatusExt for Status {
    fn set_masa_context(&mut self, ctx: &Context) {
        set_masa_context_in_metadata(self.metadata_mut(), ctx);
    }

    fn with_masa_context(mut self, ctx: &Context) -> Self {
        set_masa_context_in_metadata(self.metadata_mut(), ctx);
        self
    }

    fn get_masa_context(&self) -> Option<Context> {
        get_masa_context_from_metadata(self.metadata())
    }
}

#[allow(dead_code)]
fn read_context<B>(req: &http::Request<B>) -> Context {
    let ctx_str = req.headers()[MASA_CONTEXT_HEADER].to_str().unwrap();
    Context::from_header_string(ctx_str)
}

/// Resolve the method name from HTTP request headers, checking for override header.
pub(crate) fn resolve_method_name_from_http<B>(
    method: GrpcMethod,
    req: &http::Request<B>,
) -> CowGrpcMethod {
    if let Some(header_value) = req.headers().get(METHOD_NAME_OVERRIDE_HEADER) {
        if let Ok(method_name) = header_value.to_str() {
            if let Some(service_header) = req.headers().get(SERVICE_NAME_OVERRIDE_HEADER) {
                if let Ok(service_name) = service_header.to_str() {
                    return CowGrpcMethod::new(service_name.to_string(), method_name.to_string());
                }
            }
            return CowGrpcMethod::new(method.service(), method_name.to_string());
        }
    }
    CowGrpcMethod::new(method.service(), method.method())
}

/// Resolve the method name from Request metadata, checking for override header.
pub(crate) fn resolve_method_name_from_request<T>(
    method: GrpcMethod,
    request: &Request<T>,
) -> CowGrpcMethod {
    if let Some(header_value) = request.metadata().get(METHOD_NAME_OVERRIDE_HEADER) {
        if let Ok(method_name) = header_value.to_str() {
            if let Some(service_header) = request.metadata().get(SERVICE_NAME_OVERRIDE_HEADER) {
                if let Ok(service_name) = service_header.to_str() {
                    return CowGrpcMethod::new(service_name.to_string(), method_name.to_string());
                }
            }
            return CowGrpcMethod::new(method.service(), method_name.to_string());
        }
    }
    CowGrpcMethod::new(method.service(), method.method())
}

//! Masa context extension traits and helpers.
//!
//! These were originally in `tonic::masa_ext` but are moved here to avoid
//! a circular dependency between tonic and masa-policy. They depend on
//! `masa_core::Context` for serialization.

use masa_core::Context;
use tonic_core::metadata::{Ascii, MetadataValue};
use tonic_core::{Request, Response, Status};

/// Internal header key for MASA context.
pub const MASA_CONTEXT_HEADER: &str = masa_core::MASA_CONTEXT_HEADER;

/// Get the MASA context from metadata.
pub fn get_masa_context_from_metadata(
    metadata: &tonic_core::metadata::MetadataMap,
) -> Option<Context> {
    metadata
        .get(MASA_CONTEXT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(Context::from_header_string)
}

/// Set the MASA context in metadata.
pub fn set_masa_context_in_metadata(
    metadata: &mut tonic_core::metadata::MetadataMap,
    ctx: &Context,
) {
    let value: MetadataValue<Ascii> = ctx.to_header_string().parse().unwrap();
    metadata.insert(MASA_CONTEXT_HEADER, value);
}

/// Read the MASA context from the HTTP request headers.
pub fn read_context<B>(req: &http::Request<B>) -> Context {
    let ctx_str = req.headers()[MASA_CONTEXT_HEADER].to_str().unwrap();
    Context::from_header_string(ctx_str)
}

/// Extension trait for `Request<T>` to set the method name override header.
pub trait MasaRequestExt<T> {
    /// Set the method name override header on this request.
    fn set_method_name_override(&mut self, method_name: &str) -> Result<(), Status>;

    /// Set the service name override header on this request.
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
        use tonic_core::masa_ext::METHOD_NAME_OVERRIDE_HEADER;
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
        use tonic_core::masa_ext::SERVICE_NAME_OVERRIDE_HEADER;
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

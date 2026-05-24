//! Masa context extension traits and helpers.

use masa_core::Context;
use tonic_core::metadata::{Ascii, MetadataValue};
use tonic_core::{Request, Response, Status};

/// Internal header key for MASA context.
pub const MASA_CONTEXT_HEADER: &str = masa_core::MASA_CONTEXT_HEADER;

/// Get the MASA context from metadata.
pub fn get_masa_context_from_metadata(
    metadata: &tonic_core::metadata::MetadataMap,
) -> Option<Context> {
    metadata.get(MASA_CONTEXT_HEADER).map(|value| {
        let ctx_str = value.to_str().unwrap_or_else(|err| {
            panic!(
                "{}",
                masa_core::invalid_context_header_metadata_message(err)
            )
        });
        Context::from_header_string(ctx_str)
    })
}

/// Set the MASA context in metadata.
pub fn set_masa_context_in_metadata(
    metadata: &mut tonic_core::metadata::MetadataMap,
    ctx: &Context,
) {
    let value: MetadataValue<Ascii> = ctx.to_header_string().parse().unwrap();
    metadata.insert(MASA_CONTEXT_HEADER, value);
}

pub use masa_core::{read_context, read_context_from_headers, read_priority_from_headers};

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
        use masa_tonic_core::METHOD_NAME_OVERRIDE_HEADER;
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
        use masa_tonic_core::SERVICE_NAME_OVERRIDE_HEADER;
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

#[cfg(test)]
mod tests {
    use super::*;
    use masa_core::ContextBuilder;

    #[test]
    fn request_extension_round_trips_context() {
        let ctx = ContextBuilder::new("test.Service", 7)
            .slo(100)
            .gateway_entry(10)
            .deadline(110)
            .build();
        let mut request = Request::new(());

        request.set_masa_context(&ctx);

        assert_eq!(
            request
                .get_masa_context()
                .expect("missing context")
                .deadline(),
            ctx.deadline()
        );
    }

    #[test]
    fn response_extension_round_trips_context() {
        let ctx = ContextBuilder::new("test.Service", 8)
            .slo(100)
            .gateway_entry(10)
            .deadline(110)
            .build();
        let response = Response::new(()).with_masa_context(&ctx);

        assert_eq!(
            response
                .get_masa_context()
                .expect("missing context")
                .request_id(),
            ctx.request_id()
        );
    }
}

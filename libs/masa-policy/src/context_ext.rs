//! Masa context extension traits and helpers.
//!
//! These were originally in `tonic::masa_ext` but are moved here to avoid
//! a circular dependency between tonic and masa-policy. They depend on
//! `masa_core::Context` for serialization and are reexported through
//! `tonic::masa_ext` for compatibility with existing application imports.

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

/// Read the MASA context from HTTP headers.
pub fn read_context_from_headers(headers: &http::HeaderMap) -> Context {
    let ctx = headers
        .get(MASA_CONTEXT_HEADER)
        .unwrap_or_else(|| panic!("{}", masa_core::MISSING_CONTEXT_HEADER_MESSAGE));
    let ctx_str = ctx.to_str().unwrap_or_else(|err| {
        panic!(
            "{}",
            masa_core::invalid_context_header_metadata_message(err)
        )
    });
    Context::from_header_string(ctx_str)
}

/// Read the MASA context from the HTTP request headers.
pub fn read_context<B>(req: &http::Request<B>) -> Context {
    read_context_from_headers(req.headers())
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
    use http::HeaderValue;
    use masa_core::{ContextBuilder, PriorityHint};

    #[test]
    #[should_panic(expected = "missing MASA context header `ctx`")]
    fn read_context_panics_with_explicit_message_when_missing() {
        let req = http::Request::new(());

        let _ = read_context(&req);
    }

    #[test]
    #[should_panic(expected = "invalid MASA context header `ctx`: invalid ASCII/metadata")]
    fn read_context_panics_with_explicit_message_for_invalid_ascii() {
        let mut req = http::Request::new(());
        req.headers_mut().insert(
            MASA_CONTEXT_HEADER,
            HeaderValue::from_bytes(b"\xff").unwrap(),
        );

        let _ = read_context(&req);
    }

    #[test]
    #[should_panic(expected = "invalid MASA context header `ctx`: invalid base64")]
    fn read_context_panics_with_explicit_message_for_invalid_base64() {
        let mut req = http::Request::new(());
        req.headers_mut()
            .insert(MASA_CONTEXT_HEADER, HeaderValue::from_static("not-base64"));

        let _ = read_context(&req);
    }

    #[test]
    fn read_context_preserves_valid_context() {
        let ctx = ContextBuilder::new("test.Service", 7)
            .slo(100)
            .gateway_entry(10)
            .deadline(110)
            .build();
        let req = http::Request::builder()
            .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
            .body(())
            .unwrap();

        let decoded = read_context(&req);

        assert_eq!(decoded.api(), ctx.api());
        assert_eq!(decoded.request_id(), ctx.request_id());
        assert_eq!(decoded.deadline(), ctx.deadline());
    }

    #[test]
    #[should_panic(expected = "missing MASA context header `ctx`")]
    fn read_context_from_headers_panics_with_explicit_message_when_missing() {
        let headers = http::HeaderMap::new();

        let _ = read_context_from_headers(&headers);
    }

    #[test]
    #[should_panic(expected = "invalid MASA context header `ctx`: invalid ASCII/metadata")]
    fn read_context_from_headers_panics_with_explicit_message_for_invalid_ascii() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            MASA_CONTEXT_HEADER,
            HeaderValue::from_bytes(b"\xff").unwrap(),
        );

        let _ = read_context_from_headers(&headers);
    }

    #[test]
    fn read_context_from_headers_preserves_priority_hint() {
        let ctx = ContextBuilder::new("test.Service", 9)
            .slo(100)
            .gateway_entry(10)
            .deadline(110)
            .prio_hint(PriorityHint::new(42))
            .build();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            MASA_CONTEXT_HEADER,
            HeaderValue::from_str(&ctx.to_header_string()).unwrap(),
        );

        assert_eq!(
            read_context_from_headers(&headers).prio_hint(),
            PriorityHint::new(42)
        );
    }
}

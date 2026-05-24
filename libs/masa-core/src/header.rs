use crate::{Context, PriorityHint, MASA_CONTEXT_HEADER};

/// Read the MASA context from HTTP headers.
pub fn read_context_from_headers(headers: &http::HeaderMap) -> Context {
    let ctx = headers
        .get(MASA_CONTEXT_HEADER)
        .unwrap_or_else(|| panic!("{}", crate::MISSING_CONTEXT_HEADER_MESSAGE));
    let ctx_str = ctx
        .to_str()
        .unwrap_or_else(|err| panic!("{}", crate::invalid_context_header_metadata_message(err)));
    Context::from_header_string(ctx_str)
}

/// Read the MASA context from HTTP request headers.
pub fn read_context<B>(req: &http::Request<B>) -> Context {
    read_context_from_headers(req.headers())
}

/// Read the priority hint from MASA context HTTP headers.
pub fn read_priority_from_headers(headers: &http::HeaderMap) -> PriorityHint {
    read_context_from_headers(headers).prio_hint()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextBuilder;
    use http::HeaderValue;

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
    fn read_priority_from_headers_preserves_priority_hint() {
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

        assert_eq!(read_priority_from_headers(&headers), PriorityHint::new(42));
    }
}

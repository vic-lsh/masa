use masa::{
    get_masa_context_from_metadata, read_context, set_masa_context_in_metadata, MasaRequestExt,
    MasaResponseExt, MasaStatusExt, MASA_CONTEXT_HEADER,
};
use masa_core::{Context, ContextBuilder};
use tonic::{Request, Response, Status};

fn test_context(request_id: u64) -> Context {
    ContextBuilder::new("metadata.Echo", request_id)
        .slo(1_000)
        .gateway_entry(1)
        .deadline(2)
        .build()
}

fn assert_context_matches(actual: Context, expected: &Context) {
    assert_eq!(actual.api(), expected.api());
    assert_eq!(actual.request_id(), expected.request_id());
    assert_eq!(actual.deadline(), expected.deadline());
}

#[test]
fn metadata_helpers_round_trip_without_network() {
    let mut metadata = tonic::metadata::MetadataMap::new();
    let ctx = test_context(9);

    assert!(get_masa_context_from_metadata(&metadata).is_none());

    set_masa_context_in_metadata(&mut metadata, &ctx);

    let extracted = get_masa_context_from_metadata(&metadata).expect("missing context");
    assert_context_matches(extracted, &ctx);
}

#[test]
fn masa_reexports_request_response_status_helpers() {
    let ctx = test_context(10);

    let mut request = Request::new(());
    request.set_masa_context(&ctx);
    assert_context_matches(
        request.get_masa_context().expect("missing request context"),
        &ctx,
    );

    let request = Request::new(()).with_masa_context(&ctx);
    assert_context_matches(
        request.get_masa_context().expect("missing request context"),
        &ctx,
    );

    let mut response = Response::new(());
    response.set_masa_context(&ctx);
    assert_context_matches(
        response
            .get_masa_context()
            .expect("missing response context"),
        &ctx,
    );

    let response = Response::new(()).with_masa_context(&ctx);
    assert_context_matches(
        response
            .get_masa_context()
            .expect("missing response context"),
        &ctx,
    );

    let mut status = Status::internal("error");
    status.set_masa_context(&ctx);
    assert_context_matches(
        status.get_masa_context().expect("missing status context"),
        &ctx,
    );

    let status = Status::internal("error").with_masa_context(&ctx);
    assert_context_matches(
        status.get_masa_context().expect("missing status context"),
        &ctx,
    );
}

#[test]
fn masa_reexports_http_header_reader() {
    let ctx = test_context(11);
    let request = http::Request::builder()
        .header(MASA_CONTEXT_HEADER, ctx.to_header_string())
        .body(())
        .unwrap();

    assert_context_matches(read_context(&request), &ctx);
}

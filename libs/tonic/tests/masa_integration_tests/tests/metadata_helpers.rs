use masa_core::ContextBuilder;
use tonic::masa::context::{get_masa_context_from_metadata, set_masa_context_in_metadata};

#[test]
fn metadata_helpers_round_trip_without_network() {
    let mut metadata = tonic::metadata::MetadataMap::new();
    let ctx = ContextBuilder::new("metadata.Echo", 9)
        .slo(1_000)
        .gateway_entry(1)
        .deadline(2)
        .build();

    assert!(get_masa_context_from_metadata(&metadata).is_none());

    set_masa_context_in_metadata(&mut metadata, &ctx);

    let extracted = get_masa_context_from_metadata(&metadata).expect("missing context");
    assert_eq!(extracted.api(), ctx.api());
    assert_eq!(extracted.request_id(), ctx.request_id());
    assert_eq!(extracted.deadline(), ctx.deadline());
}

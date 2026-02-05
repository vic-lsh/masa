use masa_core::{Context, ContextBuilder, PriorityHint, QueueLatencies};

fn sample_context() -> Context {
    ContextBuilder::new("hotel.SearchHotels", 42)
        .slo(500_000)
        .gateway_entry(1_000_000)
        .deadline(1_500_000)
        .prio_hint(PriorityHint::new(1_250_000))
        .frontend_elapse(12_345)
        .queue_latencies(QueueLatencies {
            initial: 7,
            resume: 11,
        })
        .build()
}

#[test]
fn header_round_trip_preserves_all_fields() {
    let ctx = sample_context();

    let encoded = ctx.to_header_string();
    assert!(!encoded.is_empty());

    let decoded = Context::from_header_string(&encoded);

    assert_eq!(decoded.api(), ctx.api());
    assert_eq!(decoded.request_id(), ctx.request_id());
    assert_eq!(decoded.slo(), ctx.slo());
    assert_eq!(decoded.gateway_entry(), ctx.gateway_entry());
    assert_eq!(decoded.deadline(), ctx.deadline());
    assert_eq!(decoded.prio_hint(), ctx.prio_hint());
    assert_eq!(decoded.frontend_elapse(), ctx.frontend_elapse());
    assert_eq!(
        decoded.queue_latencies.as_ref(),
        ctx.queue_latencies.as_ref()
    );
}

#[test]
fn json_round_trip_matches_header_round_trip() {
    let ctx = sample_context();

    let json = ctx.to_json();
    let decoded_from_json = Context::from_json(&json);
    let decoded_from_header = Context::from_header_string(&ctx.to_header_string());

    assert_eq!(decoded_from_json.api(), decoded_from_header.api());
    assert_eq!(decoded_from_json.deadline(), decoded_from_header.deadline());
    assert_eq!(
        decoded_from_json.queue_latencies.as_ref(),
        decoded_from_header.queue_latencies.as_ref()
    );
}

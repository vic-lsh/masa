#[cfg(feature = "trace_queue_latency")]
use masa_core::QueueLatencies;
use masa_core::{Context, ContextBuilder, PriorityHint};

fn sample_context() -> Context {
    let builder = ContextBuilder::new("hotel.SearchHotels", 42)
        .slo(500_000)
        .gateway_entry(1_000_000)
        .deadline(1_500_000)
        .prio_hint(PriorityHint::new(1_250_000))
        .frontend_elapse(12_345);
    #[cfg(feature = "trace_queue_latency")]
    let builder = builder.queue_latencies(QueueLatencies {
        initial: 7,
        resume: 11,
        ..Default::default()
    });
    builder.build()
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
    #[cfg(feature = "trace_queue_latency")]
    assert_eq!(decoded.queue_latencies(), ctx.queue_latencies());
}

#[test]
fn json_round_trip_matches_header_round_trip() {
    let ctx = sample_context();

    let json = ctx.to_json();
    let decoded_from_json = Context::from_json(&json);
    let decoded_from_header = Context::from_header_string(&ctx.to_header_string());

    assert_eq!(decoded_from_json.api(), decoded_from_header.api());
    assert_eq!(decoded_from_json.deadline(), decoded_from_header.deadline());
    #[cfg(feature = "trace_queue_latency")]
    assert_eq!(
        decoded_from_json.queue_latencies(),
        decoded_from_header.queue_latencies()
    );
}

#[test]
#[should_panic(expected = "invalid MASA context header `ctx`: invalid base64")]
fn malformed_header_reports_invalid_base64() {
    let _ = Context::from_header_string("not-base64");
}

#[test]
#[should_panic(expected = "invalid MASA context header `ctx`: invalid bincode payload")]
fn malformed_header_reports_invalid_bincode_payload() {
    let _ = Context::from_header_string("AA==");
}

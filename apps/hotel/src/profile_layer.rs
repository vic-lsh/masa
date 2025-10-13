use masa::FutureSpan;
use tonic::metadata::MetadataMap;

// Extract latency traces from the response headers
pub fn extract_latency_traces(metadata: &MetadataMap) -> Option<Vec<String>> {
    let header_value = metadata
        .get("X-Latency-Traces")
        .expect("missing X-Latency-Traces header")
        .to_str()
        .unwrap();
    let traces: Vec<FutureSpan> = serde_json::from_str(header_value).ok()?;
    traces
        .iter()
        .map(|span| match span {
            FutureSpan::Compute(duration) => format!("Compute({}us)", duration),
            FutureSpan::LocalBlock(duration) => format!("LocalBlock({}us)", duration),
            FutureSpan::ChildBlock(duration) => format!("ChildBlock({}us)", duration),
            FutureSpan::Queueing(duration) => format!("Queueing({}us)", duration),
        })
        .collect::<Vec<String>>()
        .into()
}

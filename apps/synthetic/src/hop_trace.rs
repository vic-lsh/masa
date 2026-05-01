use serde::{Deserialize, Serialize};
use tonic::Status;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HopTrace {
    pub service_id: String,
    pub method_name: String,
    pub sent_at: u64,
    pub started_at: u64,
    pub finished_at: u64,
    pub configured_work_us: u64,
    pub queueing_latency_us: u64,
    pub handler_latency_us: u64,
    pub children: Vec<HopTrace>,
}

pub fn encode_hop_traces(traces: &[HopTrace]) -> Result<String, Status> {
    serde_json::to_string(traces)
        .map_err(|e| Status::internal(format!("failed to encode hop trace json: {}", e)))
}

pub fn decode_hop_traces(json: &str) -> Result<Vec<HopTrace>, Status> {
    if json.is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(json)
        .map_err(|e| Status::internal(format!("failed to decode hop trace json: {}", e)))
}

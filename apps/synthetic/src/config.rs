use serde::{Deserialize, Serialize};

#[derive(Deserialize, Clone, Debug, Serialize)]
pub enum LatencyDistribution {
    Normal { mean: f64, std: f64 },
    Discrete { values: Vec<f64>, weights: Vec<f64> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntheticConfig {
    #[serde(rename = "FrontendIp")]
    pub frontend_ip: String,
    #[serde(rename = "FrontendPort")]
    pub frontend_port: u16,

    #[serde(rename = "ChildIps")]
    pub child_ips: Vec<String>,
    #[serde(rename = "ChildPorts")]
    pub child_ports: Vec<u16>,
    #[serde(rename = "ChildConstantLatency")]
    pub child_constant_latency: u64,
    #[serde(rename = "ChildConstantLatencySlowdownDuration")]
    pub child_constant_latency_slowdown_duration: u16, // ms
    #[serde(rename = "ChildRandomLatency")]
    pub child_random_latency: LatencyDistribution,
}

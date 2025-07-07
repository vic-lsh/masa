use serde::{Deserialize, Serialize};

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
    #[serde(rename = "ChildRandomMean")]
    pub child_random_mean: u64,
    #[serde(rename = "ChildRandomStd")]
    pub child_random_std: u64,
}

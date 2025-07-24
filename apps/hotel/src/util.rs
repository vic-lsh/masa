use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservationRequestStats {
    pub latency: u64,
    pub queueing_latency: u64,
    pub reservation_requests: u64,
    pub reservation_mc_misses: u64,
    pub reservation_total_mc_latency: u64,
    pub reservation_total_mongo_latency: u64,
    pub capacity_requests: u64,
    pub capacity_mc_misses: u64,
    pub capacity_total_mc_latency: u64,
    pub capacity_total_mongo_latency: u64,
    pub mc_bulk_insert_latency: u64,
    pub reservation_bulk_insert_latency: u64,
    pub get_database_client: u64,
    pub get_mc_client: u64,
}

impl ReservationRequestStats {
    pub fn new() -> Self {
        Self {
            latency: 0,
            queueing_latency: 0,
            reservation_total_mc_latency: 0,
            reservation_requests: 0,
            reservation_mc_misses: 0,
            reservation_total_mongo_latency: 0,
            capacity_requests: 0,
            capacity_mc_misses: 0,
            capacity_total_mc_latency: 0,
            capacity_total_mongo_latency: 0,
            mc_bulk_insert_latency: 0,
            reservation_bulk_insert_latency: 0,
            get_database_client: 0,
            get_mc_client: 0,
        }
    }
}

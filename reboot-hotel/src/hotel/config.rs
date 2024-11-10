use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(rename = "Repeats")]
    pub repeats: u64,
    #[serde(rename = "Slo")]
    pub slo: u64,
    #[serde(rename = "Rps")]
    pub rps_values: Vec<u64>,
    #[serde(rename = "WarmupSecs")]
    pub warmup_secs: u64,
    #[serde(rename = "DurationSecs")]
    pub duration_secs: u64,
    #[serde(rename = "Concurrency")]
    pub concurrency: usize,
    #[serde(rename = "Output")]
    pub output: String,
    #[serde(rename = "Addr")]
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotelConfig {
    #[serde(rename = "Hotels")]
    pub hotels: u32,
    #[serde(rename = "Payload")]
    pub payload: u32,
    #[serde(rename = "CacheConns")]
    pub cache_conns: u32,
    #[serde(rename = "ProbCacheMiss")]
    pub prob_cache_miss: u32,

    // #[serde(rename = "FrontendPort")]
    // pub frontend_port: String,

    // #[serde(rename = "SearchAddr")]
    // pub search_addr: String,
    // #[serde(rename = "SearchPort")]
    // pub search_port: String,

    // #[serde(rename = "GeoAddr")]
    // pub geo_addr: String,
    // #[serde(rename = "GeoPort")]
    // pub geo_port: String,
    #[serde(rename = "GeoRange")]
    pub geo_range: u32,

    // #[serde(rename = "RateAddr")]
    // pub rate_addr: String,
    // #[serde(rename = "RatePort")]
    // pub rate_port: String,
    #[serde(rename = "RateMongodbAddr")]
    pub rate_mongodb_addr: String,
    #[serde(rename = "RateMemcachedAddr")]
    pub rate_memcached_addr: String,

    // #[serde(rename = "ProfileAddr")]
    // pub profile_addr: String,
    // #[serde(rename = "ProfilePort")]
    // pub profile_port: String,
    #[serde(rename = "ProfileMongodbAddr")]
    pub profile_mongodb_addr: String,
    #[serde(rename = "ProfileMemcachedAddr")]
    pub profile_memcached_addr: String,

    // #[serde(rename = "ReservationAddr")]
    // pub reservation_addr: String,
    // #[serde(rename = "ReservationPort")]
    // pub reservation_port: String,
    #[serde(rename = "ReservationHotels")]
    pub reservation_hotels: u32,
    #[serde(rename = "ReservationDates")]
    pub reservation_dates: u32,
    #[serde(rename = "ReservationProbHotelAvail")]
    pub reservation_prob_hotel_avail: u32,
    #[serde(rename = "ReservationMongodbAddr")]
    pub reservation_mongodb_addr: String,
    #[serde(rename = "ReservationMemcachedAddr")]
    pub reservation_memcached_addr: String,

    #[serde(rename = "UserUsers")]
    pub user_users: u32,
    #[serde(rename = "UserProbCheckUser")]
    pub user_prob_check_user: u32,
    #[serde(rename = "UserMongodbAddr")]
    pub user_mongodb_addr: String,
}

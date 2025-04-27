use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(rename = "Repeats")]
    pub repeats: u64,
    #[serde(rename = "Apis")]
    pub apis: Vec<String>,
    #[serde(rename = "Slos")]
    pub slos: Vec<u64>,
    #[serde(rename = "Rps")]
    pub rps_values: Vec<u64>,
    #[serde(rename = "Gap")]
    pub gap: String,
    #[serde(rename = "WarmupSecs")]
    pub warmup_secs: u64,
    #[serde(rename = "DurationSecs")]
    pub duration_secs: u64,
    #[serde(rename = "Concurrency")]
    pub concurrency: usize,
    #[serde(rename = "Addr")]
    pub addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotelConfig {
    #[serde(rename = "ExecutorThreads")]
    pub executor_threads: u32,

    #[serde(rename = "Hotels")]
    pub hotels: u32,

    #[serde(rename = "CacheConns")]
    pub cache_conns: u32,
    #[serde(rename = "ProbCacheMiss")]
    pub prob_cache_miss: u32,

    #[serde(rename = "FrontendIp")]
    pub frontend_ip: String,
    #[serde(rename = "FrontendPort")]
    pub frontend_port: u16,

    #[serde(rename = "GeoIp")]
    pub geo_ip: String,
    #[serde(rename = "GeoPort")]
    pub geo_port: u16,

    #[serde(rename = "ProfileIp")]
    pub profile_ip: String,
    #[serde(rename = "ProfilePort")]
    pub profile_port: u16,

    #[serde(rename = "RateIp")]
    pub rate_ip: String,
    #[serde(rename = "RatePort")]
    pub rate_port: u16,

    #[serde(rename = "RecommendationIp")]
    pub recommendation_ip: String,
    #[serde(rename = "RecommendationPort")]
    pub recommendation_port: u16,

    #[serde(rename = "ReservationIp")]
    pub reservation_ip: String,
    #[serde(rename = "ReservationPort")]
    pub reservation_port: u16,

    #[serde(rename = "ReviewIp")]
    pub review_ip: String,
    #[serde(rename = "ReviewPort")]
    pub review_port: u16,

    #[serde(rename = "SearchIp")]
    pub search_ip: String,
    #[serde(rename = "SearchPort")]
    pub search_port: u16,

    #[serde(rename = "UserIp")]
    pub user_ip: String,
    #[serde(rename = "UserPort")]
    pub user_port: u16,

    #[serde(rename = "RateMongodbAddr")]
    pub rate_mongodb_addr: String,
    #[serde(rename = "RateMemcachedAddr")]
    pub rate_memcached_addr: String,

    #[serde(rename = "ProfileMongodbAddr")]
    pub profile_mongodb_addr: String,
    #[serde(rename = "ProfileMemcachedAddr")]
    pub profile_memcached_addr: String,

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

    // #[serde(rename = "RatePort")]
    // pub rate_port: String,
    #[serde(rename = "ReviewMongodbAddr")]
    pub review_mongodb_addr: String,
    #[serde(rename = "ReviewMemcachedAddr")]
    pub review_memcached_addr: String,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotelConfig {
    #[serde(rename = "Hotels")]
    pub hotels: u32,

    #[serde(rename = "CacheConns")]
    pub cache_conns: u32,
    #[serde(rename = "ProbCacheMiss")]
    pub prob_cache_miss: u32,

    #[serde(rename = "FrontendIp")]
    pub frontend_ip: String,
    #[serde(rename = "FrontendPort", default = "default_port")]
    pub frontend_port: u16,

    #[serde(rename = "GeoIp")]
    pub geo_ip: String,
    #[serde(rename = "GeoPort", default = "default_port")]
    pub geo_port: u16,
    #[serde(rename = "GeoReplicas", default = "one")]
    pub geo_replicas: u8,

    #[serde(rename = "ProfileIp")]
    pub profile_ip: String,
    #[serde(rename = "ProfilePort", default = "default_port")]
    pub profile_port: u16,
    #[serde(rename = "ProfileReplicas", default = "one")]
    pub profile_replicas: u8,

    #[serde(rename = "RateIp")]
    pub rate_ip: String,
    #[serde(rename = "RatePort", default = "default_port")]
    pub rate_port: u16,
    #[serde(rename = "RateReplicas", default = "one")]
    pub rate_replicas: u8,

    #[serde(rename = "RecommendationIp")]
    pub recommendation_ip: String,
    #[serde(rename = "RecommendationPort", default = "default_port")]
    pub recommendation_port: u16,
    #[serde(rename = "RecommendationReplicas", default = "one")]
    pub recommendation_replicas: u8,

    #[serde(rename = "ReservationIp")]
    pub reservation_ip: String,
    #[serde(rename = "ReservationPort", default = "default_port")]
    pub reservation_port: u16,
    #[serde(rename = "ReservationReplicas", default = "one")]
    pub reservation_replicas: u8,

    #[serde(rename = "ReviewIp")]
    pub review_ip: String,
    #[serde(rename = "ReviewPort", default = "default_port")]
    pub review_port: u16,
    #[serde(rename = "ReviewReplicas", default = "one")]
    pub review_replicas: u8,

    #[serde(rename = "SearchIp")]
    pub search_ip: String,
    #[serde(rename = "SearchPort", default = "default_port")]
    pub search_port: u16,
    #[serde(rename = "SearchReplicas", default = "one")]
    pub search_replicas: u8,

    #[serde(rename = "UserIp")]
    pub user_ip: String,
    #[serde(rename = "UserPort", default = "default_port")]
    pub user_port: u16,
    #[serde(rename = "UserReplicas", default = "one")]
    pub user_replicas: u8,

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

    #[serde(rename = "ReviewMongodbAddr")]
    pub review_mongodb_addr: String,
    #[serde(rename = "ReviewMemcachedAddr")]
    pub review_memcached_addr: String,
}

fn one() -> u8 {
    1
}

fn default_port() -> u16 {
    8000
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "Hotels")]
    pub hotels: u32,
    #[serde(rename = "Payload")]
    pub payload: u32,
    #[serde(rename = "CacheConn")]
    pub cache_conn: u32,
    #[serde(rename = "CacheMissRate")]
    pub cache_miss_rate: u32,

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
}

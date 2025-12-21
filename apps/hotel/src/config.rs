use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlobalConfig {
    pub hotels: u32,
    pub cache_conns: u32,
    pub prob_cache_miss: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasicServiceConfig {
    pub ip: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "one")]
    pub replicas: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CachedServiceConfig {
    #[serde(flatten)]
    pub endpoint: BasicServiceConfig,
    pub mongodb_addr: String,
    pub redis_addr: String,
}

impl Deref for CachedServiceConfig {
    type Target = BasicServiceConfig;

    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

impl DerefMut for CachedServiceConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoint
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserServiceConfig {
    #[serde(flatten)]
    pub endpoint: BasicServiceConfig,
    pub mongodb_addr: String,
    pub users: u32,
    pub prob_check_user: u32,
}

impl Deref for UserServiceConfig {
    type Target = BasicServiceConfig;

    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

impl DerefMut for UserServiceConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoint
    }
}

pub type FrontendConfig = BasicServiceConfig;
pub type GeoConfig = BasicServiceConfig;
pub type ProfileConfig = CachedServiceConfig;
pub type RateConfig = CachedServiceConfig;
pub type RecommendationConfig = BasicServiceConfig;
pub type ReservationConfig = CachedServiceConfig;
pub type ReviewConfig = CachedServiceConfig;
pub type SearchConfig = BasicServiceConfig;
pub type UserConfig = UserServiceConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotelConfig {
    pub global: GlobalConfig,
    pub frontend: FrontendConfig,
    pub geo: GeoConfig,
    pub profile: ProfileConfig,
    pub rate: RateConfig,
    pub recommendation: RecommendationConfig,
    pub reservation: ReservationConfig,
    pub review: ReviewConfig,
    pub search: SearchConfig,
    pub user: UserConfig,
}

fn one() -> u8 {
    1
}

fn default_port() -> u16 {
    8000
}

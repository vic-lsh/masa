use anyhow::Result;
use crate::{RedisPool, SocialGraphClient};

pub async fn run_worker(_id: usize, _redis_pool: RedisPool, _sg_client: SocialGraphClient) -> Result<()> {
    Ok(())
}

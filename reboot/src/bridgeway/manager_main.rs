mod manager;

use std::error::Error;

use manager::Manager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let n_hotels = 10_000;
    let payload = 16;
    let cache_addr = "memcache://127.0.0.1:11003".to_string();
    let cache_conn = 32;
    let cache_miss_rate = 0.5;
    let db_addr = "mongodb://127.0.0.1:27003".to_string();

    let manager = Manager::new(
        n_hotels,
        payload,
        cache_addr,
        cache_conn,
        cache_miss_rate,
        db_addr,
    )
    .await?;

    let mut names = Vec::new();
    for i in 0..3 {
        names.push(format!("Tung Chung Ave {}", 3667 + i));
    }
    let hotels = manager.fetch_mixture(names).await;

    println!("hotels: {:?}", hotels);

    Ok(())
}

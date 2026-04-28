use socialnet::post_storage::server::{run, Args};

#[cfg_attr(feature = "sched_mt", tokio::main)]
#[cfg_attr(not(feature = "sched_mt"), tokio::main(flavor = "current_thread"))]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::from_env()?;
    run(args).await
}

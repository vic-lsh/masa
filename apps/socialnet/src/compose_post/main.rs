use socialnet::compose_post::server::{run, Args};

#[cfg(feature = "sched_mt")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    app_utils::runtime::block_on(main_inner())
}

#[cfg(not(feature = "sched_mt"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    main_inner().await
}

async fn main_inner() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::from_env()?;
    run(args).await
}

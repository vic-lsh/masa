mod server;

use server::{run, Args};

use tracing::Level;
use tracing_subscriber::FmtSubscriber;

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
    // Replaced env_logger with tracing for consistency
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    let args = Args::from_env()?;
    run(args).await
}

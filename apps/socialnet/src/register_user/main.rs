use socialnet::register_user::server::{run, Args};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let args = Args::from_env()?;
    run(args).await
}

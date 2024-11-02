pub mod hotel {
    tonic::include_proto!("frontend");
}
mod gen;

use tonic::Request;
use tonic_masa::{Context, GraphId};

use gen::gen_search_request;
use reboot_hotel::init_logging;

use hotel::frontend_client::FrontendClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let mut client = FrontendClient::connect("http://[::1]:8660").await?;

    let graph_id: GraphId = "Hotel".to_string();
    let test_id = 0;
    let request_id = 2024;
    let slo = 10_000;
    let request_class = 0;
    let start_at = 0;
    let deadline = 10_000;
    let latest_exec_at = deadline;

    let ctx = Context::new(
        graph_id.clone(),
        test_id,
        request_id,
        slo,
        request_class,
        start_at,
        deadline,
        latest_exec_at,
    );

    let mut request = Request::new(gen_search_request());
    request.metadata_mut().insert_ctx("ctx", &ctx);

    let response = client.handle_search(request).await?;

    log::info!("{:?}", response);

    Ok(())
}

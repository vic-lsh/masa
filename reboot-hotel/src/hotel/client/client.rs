pub mod hotel {
    tonic::include_proto!("frontend");
}

use tonic::Request;
use tonic_masa::{Context, GraphId};

use reboot_hotel::init_logging;

use hotel::frontend_client::FrontendClient;
use hotel::SearchRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let mut client = FrontendClient::connect("http://[::1]:8660").await?;

    let graph_id: GraphId = "Hotel".to_string();
    let request_id = 2024;
    let deadline = 10_000;
    let latest_exec_at = deadline;
    let request_class = 0;

    let ctx = Context::new(
        graph_id.clone(),
        request_id,
        deadline,
        latest_exec_at,
        request_class,
    );

    let mut request = Request::new(SearchRequest { ave: 61 });
    request.metadata_mut().insert_ctx("ctx", &ctx);

    let response = client.handle_search(request).await?;

    log::info!("{:?}", response);

    Ok(())
}

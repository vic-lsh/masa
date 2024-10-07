use masa::frontend_client::FrontendClient;
use masa::SearchRequest;
use tonic::Request;

pub mod masa {
    tonic::include_proto!("frontend");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = FrontendClient::connect("http://node1:8660").await?;

    let request = Request::new(SearchRequest { ave: 61 });

    let response = client.handle_search(request).await?;

    println!("{:?}", response);

    Ok(())
}

#![allow(dead_code)]

use text_service::text_service_client::TextServiceClient;
use text_service::TextRequest;

pub mod text_service {
    tonic::include_proto!("textservice");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = TextServiceClient::connect("http://[::1]:50051").await?;

    let text_eq = TextRequest{text:"@john @bob Hello! Here is the link, https://openai.com".to_string()};
    let request: tonic::Request<TextRequest> = tonic::Request::new(text_eq);

    let response = client.compose_text(request).await?;
    println!("RESPONSE={:?}", response);
    Ok(())
}

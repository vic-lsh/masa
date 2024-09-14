use unique_id_service::greeter_client::GreeterClient;
use unique_id_service::UniqueIdRequest;

pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = GreeterClient::connect("http://[::1]:50051").await?;

    let request: tonic::Request<UniqueIdRequest> = tonic::Request::new(UniqueIdRequest { id: 1 });
    let response = client.compose_unique_id(request).await?;

    println!("RESPONSE={:?}", response);

    Ok(())
}

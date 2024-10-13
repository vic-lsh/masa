use recommendation::recommendation_client::RecommendationClient;
use recommendation::RecommendationRequest;

pub mod recommendation {
    tonic::include_proto!("recommendation");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = RecommendationClient::connect("http://[::1]:50051").await?;

    let request = tonic::Request::new(RecommendationRequest {
        require: "Tonic".into(),
        lat: 0.1,
        lon: 0.2,
    });

    let response = client.get_recommendations(request).await?;

    println!("RESPONSE={:?}", response);

    Ok(())
}

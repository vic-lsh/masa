use tonic::{transport::Server, Request, Response, Status};

use recommendation::recommendation_server::{Recommendation, RecommendationServer};
use recommendation::{HelloReply, HelloRequest};

pub mod recommendation {
    tonic::include_proto!("recommendation");
}

#[derive(Debug, Default)]
pub struct MyRecommendation {}

#[tonic::async_trait]
impl Recommendation for MyRecommendation {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        println!("Got a request: {:?}", request);

        let reply = HelloReply {
            hotel_ids: vec![format!("require: {}!", request.into_inner().require)],
        };

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyRecommendation::default();

    Server::builder()
        .add_service(RecommendationServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}

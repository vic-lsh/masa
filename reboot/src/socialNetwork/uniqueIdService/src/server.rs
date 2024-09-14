use tonic::{transport::Server, Request, Response, Status};

use unique_id_service::greeter_server::{Greeter, GreeterServer};
use unique_id_service::{UniqueIdReply, UniqueIdRequest};

pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn compose_unique_id(
        &self,
        request: Request<UniqueIdRequest>,
    ) -> Result<Response<UniqueIdReply>, Status> {
        println!("Got a request: {:?}", request);
        let reply = UniqueIdReply {
            message: request.into_inner().id,
        };

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let greeter = MyGreeter::default();

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}

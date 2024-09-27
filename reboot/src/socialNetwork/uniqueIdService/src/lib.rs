mod server;
use server::MyGreeter;
use tonic::{transport::Server, Request, Response, Status};
use unique_id_service::greeter_client::GreeterClient;
use unique_id_service::greeter_server::{Greeter, GreeterServer};
use unique_id_service::UniqueIdRequest;
pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unique_timestamp() {
        let server_thread = tokio::spawn(async {
            env_logger::init();
            let addr = "[::1]:50051".parse().unwrap();
            let netif = "";
            let machine_id = String::from("abc");
            let greeter = MyGreeter::new(machine_id);
            use Greeter;
            Server::builder()
                .add_service(GreeterServer::new(greeter))
                .serve(addr)
                .await
                .unwrap();
        });

        let client_thread = tokio::spawn(async {
            let mut client = GreeterClient::connect("http://[::1]:50051").await.unwrap();

            let request: tonic::Request<UniqueIdRequest> =
                tonic::Request::new(UniqueIdRequest { id: 1 });

            let response = client.compose_unique_id(request).await.unwrap();

            println!("RESPONSE={:?}", response);

            response.into_inner().message
        });
    }
}

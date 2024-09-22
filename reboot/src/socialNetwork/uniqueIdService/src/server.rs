use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use tonic::{transport::Server, Request, Response, Status};

use unique_id_service::greeter_server::{Greeter, GreeterServer};
use unique_id_service::{UniqueIdReply, UniqueIdRequest};

pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[derive(Debug, Default)]
pub struct MyGreeter {
    machine_id: String,
}

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
        // Initialize the random number generator
        let _machine_id = self.machine_id.clone();

        // Get the current system time
        let start = SystemTime::now();

        // Calculate the duration since the UNIX epoch
        let duration = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");

        // Get the timestamp in milliseconds
        let timestamp: i64 = duration.as_millis() as i64;

        let counter_hex = "1";

        // Generate a random ID between 1 and 100 (inclusive)
        let random_id = format!("{}{}{}", _machine_id, timestamp_hex, counter_hex);

        Ok(Response::new(reply))
    }
}

fn get_machine_id(netif: &str) -> String {
    todo!()
}

fn hash_mac_address_pid(mac: &str) -> u16 {
    let mut hash: u16 = 0;
    let pid = process::id().to_string();
    let mac_pid = format!("{}{}", mac, pid);
    for (i, byte) in mac_pid.bytes().enumerate() {
        hash += (byte as u16) << ((i & 1) * 8);
    }
    hash
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let netif = "";
    let greeter = MyGreeter {
        machine_id: get_machine_id(netif),
    };

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}

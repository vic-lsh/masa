use log::error;
use std::fmt::Write; // For using the `write!` macro
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH}; // Assuming you're using a logging crate like `log`

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
    let mac_addr_filename = format!("/sys/class/net/{}/address", netif);

    // Step1: open file
    let mac_addr_file = match File::open(&mac_addr_filename) {
        Ok(file) => file,
        Err(e) => {
            error!(
                "Cannot read MAC address from net interface {}: {}",
                netif, e
            );
            // return "" ?
            return String::new(); // Return empty string
        }
    };

    // Step2: read the file content into the `mac` variable
    let mut reader = BufReader::new(mac_addr_file); // Wrap the file in a buffered reader
    let mut mac = String::new();
    match reader.read_to_string(&mut mac) {
        Ok(_) => (),
        Err(e) => {
            error!("Failed to read from file: {}", e);
            return String::new(); // Return empty string
        }
    }

    // Step3: convert mac address to a string
    let mut stream = String::new();
    write!(&mut stream, "{:x}", hash_mac_address_pid(&mac)).unwrap();

    // Step4: fix the size of mac_hash to be 3
    let mut mac_hash = stream;
    if mac_hash.len() > 3 {
        mac_hash = mac_hash[mac_hash.len() - 3..].to_string(); // Slice the last 3 characters
    } else if mac_hash.len() < 3 {
        let padding = "0".repeat(3 - mac_hash.len()); // Create the necessary padding with '0's
        mac_hash = format!("{}{}", padding, mac_hash); // Prepend the padding
    }
    mac_hash
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
    env_logger::init();
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

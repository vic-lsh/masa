use core::time;
use env_logger::fmt::Timestamp;
use log::error;
use std::fmt::Write; // For using the `write!` macro
use std::fs::File;
use std::io::{BufReader, Read};
use std::process;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const CUSTOM_EPOCH: i64 = 1514764800000;
static mut CURRENT_TIMESTAMP: i64 = -1;
static mut COUNTER: i64 = 0;
use tonic::{transport::Server, Request, Response, Status};

use unique_id_service::greeter_server::{Greeter, GreeterServer};
use unique_id_service::{UniqueIdReply, UniqueIdRequest};

pub mod unique_id_service {
    tonic::include_proto!("uniqueidservice");
}

#[derive(Debug, Default)]
pub struct MyGreeter {
    machine_id: String,
    counter: Arc<Mutex<Counter>>,
}

#[derive(Debug, Default)]
struct Counter {
    current_stamp: i64,
    counter: i64,
}

impl Counter {
    fn get_counter(&mut self, timestamp: i64) -> i64 {
        if self.current_stamp > timestamp {
            eprintln!("Timestamps are not incremental.");
            std::process::exit(1);
        } else if self.current_stamp == timestamp {
            self.counter += 1;
            return self.counter - 1;
        } else {
            self.current_stamp = timestamp;
            self.counter = 1;
            return 0;
        }
    }
}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn compose_unique_id(
        &self,
        request: Request<UniqueIdRequest>,
    ) -> Result<Response<UniqueIdReply>, Status> {
        println!("Got a request: {:?}", request);

        // Part1: thread lock
        // - Get Timestamp and idx
        let timestamp: i64;
        let idx: i64;
        {
            let mut counter = self.counter.lock().unwrap();
            // Lock the mutex
            // Get the current system time (like duration_cast in C++)
            let now = SystemTime::now();
            let since_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
            timestamp = since_epoch.as_millis() as i64 - CUSTOM_EPOCH;
            idx = counter.get_counter(timestamp);
        }
        // Part2:
        // Change timestamp to a 16 hex string
        // Fix size to 10
        let timestamp_hex = get_timestamp_hex(timestamp);

        // Part3:
        // Do the same thing for idx
        // Fix size to 3
        let mut counter_hex = String::new();
        write!(&mut counter_hex, "{:x}", idx).unwrap();

        if counter_hex.len() > 3 {
            counter_hex = counter_hex[counter_hex.len() - 3..].to_string();
        } else if counter_hex.len() < 3 {
            let padding = "0".repeat(3 - counter_hex.len()); // Create the necessary padding with '0's
            counter_hex = format!("{}{}", padding, counter_hex); // Prepend the padding
        }

        // Part4: assign value to _machine_id
        // Initialize the random number generator
        let _machine_id = self.machine_id.clone();

        let post_id_str = format!("{}{}{}", _machine_id, timestamp_hex, counter_hex);
        // When you apply the bitmask 0x7FFFFFFFFFFFFFFF,
        // you are limiting the result to a 63-bit integer
        // (since 0x7FFFFFFFFFFFFFFF is the maximum value for a 63-bit unsigned integer).
        let post_id = u64::from_str_radix(&post_id_str, 16).unwrap() & 0x7FFFFFFFFFFFFFFF;
        let reply = UniqueIdReply { message: post_id };
        Ok(Response::new(reply))
    }
}

impl MyGreeter {
    pub fn new(machine_id: String) -> Self {
        MyGreeter {
            // machine_id: get_machine_id(netif),
            // now it is hardcoded
            machine_id,
            counter: Arc::new(Mutex::new(Counter::default())),
        }
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

fn get_timestamp_hex(timestamp: i64) -> String {
    // Part2:
    // Change timestamp to a 16 hex string
    // Fix size to 10
    let mut timestamp_hex = String::new();
    write!(&mut timestamp_hex, "{:x}", timestamp).unwrap();

    if timestamp_hex.len() > 10 {
        timestamp_hex = timestamp_hex[timestamp_hex.len() - 10..].to_string();
    // Slice the last 3 characters
    } else if timestamp_hex.len() < 10 {
        let padding = "0".repeat(10 - timestamp_hex.len()); // Create the necessary padding with '0's
        timestamp_hex = format!("{}{}", padding, timestamp_hex); // Prepend the padding
    }
    timestamp_hex
}

/* produces a 16-bit hash value by combining the MAC address and process ID,
    ensuring a unique result for different processes on the same machine.
*/
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
        // machine_id: get_machine_id(netif),
        // now it is hardcoded
        machine_id: String::from("abc"),
        counter: Arc::new(Mutex::new(Counter::default())),
    };

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_hex_test() {
        let timestamp_hex = get_timestamp_hex(212711383016);
        assert_eq!(timestamp_hex, "3186961fe8");
        let timestamp_hex2 = get_timestamp_hex(212711383016122);
        assert_eq!(timestamp_hex2, "75ba6ca2ba");
        let timestamp_hex3 = get_timestamp_hex(123);
        assert_eq!(timestamp_hex3, "000000007b");
    }
}

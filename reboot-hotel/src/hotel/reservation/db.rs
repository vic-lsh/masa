use mongodb::{bson::doc, options::ClientOptions, Client as MongoClient, Collection};
use serde::{Deserialize, Serialize};
use std::error::Error;
use tracing::{error, info};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Reservation {
    #[serde(rename = "hotelId")]
    pub(crate) hotel_id: String,
    #[serde(rename = "customerName")]
    pub(crate) customer_name: String,
    #[serde(rename = "inDate")]
    pub(crate) in_date: String,
    #[serde(rename = "outDate")]
    pub(crate) out_date: String,
    pub(crate) number: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Number {
    #[serde(rename = "hotelId")]
    pub(crate) hotel_id: String,
    #[serde(rename = "numberOfRoom")]
    pub(crate) number: i32,
}

fn generate_static_data() -> (Vec<Reservation>, Vec<Number>) {
    info!("Generating test data...");

    let new_reservations = vec![Reservation {
        hotel_id: "4".to_string(),
        customer_name: "Alice".to_string(),
        in_date: "2015-04-09".to_string(),
        out_date: "2015-04-10".to_string(),
        number: 1,
    }];

    let mut new_numbers = vec![
        Number {
            hotel_id: "1".to_string(),
            number: 200,
        },
        Number {
            hotel_id: "2".to_string(),
            number: 200,
        },
        Number {
            hotel_id: "3".to_string(),
            number: 200,
        },
        Number {
            hotel_id: "4".to_string(),
            number: 200,
        },
        Number {
            hotel_id: "5".to_string(),
            number: 200,
        },
        Number {
            hotel_id: "6".to_string(),
            number: 200,
        },
    ];

    // Add hotels 7-80
    for i in 7..=80 {
        let hotel_id = i.to_string();

        let room_number = match i % 3 {
            1 => 300,
            2 => 250,
            _ => 200,
        };

        new_numbers.push(Number {
            hotel_id,
            number: room_number,
        });
    }

    (new_reservations, new_numbers)
}

pub async fn initialize_database(url: &str) -> Result<MongoClient, Box<dyn Error>> {
    let (new_reservations, new_numbers) = generate_static_data();

    let uri = format!("mongodb://{}", url);
    info!("Attempting connection to {}", uri);

    let client_options = ClientOptions::parse(&uri).await?;
    let client = MongoClient::with_options(client_options)?;
    info!("Successfully connected to MongoDB");

    let database = client.database("reservation-db");
    let res_collection: Collection<Reservation> = database.collection("reservation");
    let num_collection: Collection<Number> = database.collection("number");

    // Insert reservations
    res_collection
        .insert_many(new_reservations, None)
        .await
        .map_err(|e| {
            error!("Failed to insert reservations: {}", e);
            e
        })?;

    // Insert numbers
    num_collection
        .insert_many(new_numbers, None)
        .await
        .map_err(|e| {
            error!("Failed to insert numbers: {}", e);
            e
        })?;

    info!("Successfully inserted test data into reservation DB");

    Ok(client)
}

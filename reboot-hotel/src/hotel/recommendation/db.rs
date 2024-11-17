use anyhow::Result;
use mongodb::bson::doc;
use mongodb::{Client, Collection, Database};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Hotel {
    #[serde(rename = "hotelId")]
    pub hotel_id: String,
    pub lat: f64,
    pub lon: f64,
    pub rate: f64,
    pub price: f64,
}

pub fn generate_test_data() -> Vec<Hotel> {
    let mut hotels = vec![
        Hotel {
            hotel_id: "1".to_string(),
            lat: 37.7867,
            lon: -122.4112,
            rate: 109.00,
            price: 150.00,
        },
        Hotel {
            hotel_id: "2".to_string(),
            lat: 37.7854,
            lon: -122.4005,
            rate: 139.00,
            price: 120.00,
        },
        Hotel {
            hotel_id: "3".to_string(),
            lat: 37.7834,
            lon: -122.4071,
            rate: 109.00,
            price: 190.00,
        },
        Hotel {
            hotel_id: "4".to_string(),
            lat: 37.7936,
            lon: -122.3930,
            rate: 129.00,
            price: 160.00,
        },
        Hotel {
            hotel_id: "5".to_string(),
            lat: 37.7831,
            lon: -122.4181,
            rate: 119.00,
            price: 140.00,
        },
        Hotel {
            hotel_id: "6".to_string(),
            lat: 37.7863,
            lon: -122.4015,
            rate: 149.00,
            price: 200.00,
        },
    ];

    for i in 7..=80 {
        let hotel_id = i.to_string();

        let lat = 37.7835 + (i as f64) / 500.0 * 3.0;
        let lon = -122.41 + (i as f64) / 500.0 * 4.0;

        let (rate, rate_inc) = if i % 3 == 0 {
            match i % 5 {
                0 => (109.00, 123.17),
                1 => (120.00, 140.00),
                2 => (124.00, 144.00),
                3 => (132.00, 158.00),
                4 => (232.00, 258.00),
                _ => unreachable!(),
            }
        } else {
            (135.00, 179.00)
        };

        let hotel = Hotel {
            hotel_id,
            lat,
            lon,
            rate,
            price: rate_inc,
        };
        hotels.push(hotel);
    }

    hotels
}

#[allow(unused)]
pub async fn initialize_database(url: &str) -> Result<Client> {
    let client = Client::with_uri_str(url).await?;
    info!("New session successful...");

    let db: Database = client.database("recommendation-db");
    let collection: Collection<Hotel> = db.collection("recommendation");

    info!("Generating test data...");

    let hotels = generate_test_data();
    for hotel in hotels {
        let count = collection
            .count_documents(doc! { "hotelId": &hotel.hotel_id }, None)
            .await?;
        if count == 0 {
            collection.insert_one(hotel, None).await?;
        }
    }

    // Create index
    // collection.create_index(doc! { "hotelId": 1 }, None).await?;

    Ok(client)
}

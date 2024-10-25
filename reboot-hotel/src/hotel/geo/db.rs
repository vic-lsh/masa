use mongodb::{options::ClientOptions, Client};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Point {
    #[serde(rename = "hotelId")]
    pub pid: String,
    pub lat: f64,
    pub lon: f64,
}

pub fn generate_test_data() -> Vec<Point> {
    log::info!("Generating test data...");

    let mut new_points = vec![
        Point {
            pid: "1".to_string(),
            lat: 37.7867,
            lon: -122.4112,
        },
        Point {
            pid: "2".to_string(),
            lat: 37.7854,
            lon: -122.4005,
        },
        Point {
            pid: "3".to_string(),
            lat: 37.7854,
            lon: -122.4071,
        },
        Point {
            pid: "4".to_string(),
            lat: 37.7936,
            lon: -122.3930,
        },
        Point {
            pid: "5".to_string(),
            lat: 37.7831,
            lon: -122.4181,
        },
        Point {
            pid: "6".to_string(),
            lat: 37.7863,
            lon: -122.4015,
        },
    ];

    // Generate additional points
    for i in 7..=80 {
        let hotel_id = i.to_string();
        let lat = 37.7835 + (i as f64) / 500.0 * 3.0;
        let lon = -122.41 + (i as f64) / 500.0 * 4.0;

        new_points.push(Point {
            pid: hotel_id,
            lat,
            lon,
        });
    }

    new_points
}

#[allow(unused)]
pub async fn initialize_database(url: &str) -> Result<Client, mongodb::error::Error> {
    let uri = format!("mongodb://{}", url);
    log::info!("Attempting connection to {}", uri);

    let client_options = ClientOptions::parse(&uri).await?;
    let client = Client::with_options(client_options)?;
    log::info!("Successfully connected to MongoDB");

    let collection = client.database("geo-db").collection::<Point>("geo");
    let new_points = generate_test_data();
    collection.insert_many(&new_points, None).await?;
    log::info!("Successfully inserted test data into geo DB");

    Ok(client)
}

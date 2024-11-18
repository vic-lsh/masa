use mongodb::{bson::doc, options::ClientOptions, Client};
use serde::{Deserialize, Serialize};

use crate::server::hotel;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct RoomType {
    #[serde(rename = "bookableRate")]
    bookable_rate: f64,
    code: String,
    #[serde(rename = "roomDescription")]
    room_description: String,
    #[serde(rename = "totalRate")]
    total_rate: f64,
    #[serde(rename = "totalRateInclusive")]
    total_rate_inclusive: f64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct RatePlan {
    #[serde(rename = "hotelId")]
    hotel_id: String,
    code: String,
    #[serde(rename = "inDate")]
    in_date: String,
    #[serde(rename = "outDate")]
    out_date: String,
    #[serde(rename = "roomType")]
    room_type: RoomType,
}

impl PartialOrd for RatePlan {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.room_type
            .total_rate
            .partial_cmp(&other.room_type.total_rate)
    }
}

impl From<RoomType> for hotel::rate::RoomType {
    fn from(r: RoomType) -> Self {
        Self {
            bookable_rate: r.bookable_rate,
            total_rate: r.total_rate,
            total_rate_inclusive: r.total_rate_inclusive,
            code: r.code,
            currency: "USD".into(),
            room_description: r.room_description,
        }
    }
}

impl From<RatePlan> for hotel::rate::RatePlan {
    fn from(r: RatePlan) -> Self {
        Self {
            hotel_id: r.hotel_id,
            code: r.code,
            in_date: r.in_date,
            out_date: r.out_date,
            room_type: Some(r.room_type.into()),
        }
    }
}

fn generate_test_data() -> Vec<RatePlan> {
    log::info!("Generating test data...");

    let mut new_rate_plans: Vec<RatePlan> = vec![
        RatePlan {
            hotel_id: "1".to_string(),
            code: "RACK".to_string(),
            in_date: "2015-04-09".to_string(),
            out_date: "2015-04-10".to_string(),
            room_type: RoomType {
                bookable_rate: 109.00,
                code: "KNG".to_string(),
                room_description: "King sized bed".to_string(),
                total_rate: 109.00,
                total_rate_inclusive: 123.17,
            },
        },
        RatePlan {
            hotel_id: "2".to_string(),
            code: "RACK".to_string(),
            in_date: "2015-04-09".to_string(),
            out_date: "2015-04-10".to_string(),
            room_type: RoomType {
                bookable_rate: 139.00,
                code: "QN".to_string(),
                room_description: "Queen sized bed".to_string(),
                total_rate: 139.00,
                total_rate_inclusive: 153.09,
            },
        },
        RatePlan {
            hotel_id: "3".to_string(),
            code: "RACK".to_string(),
            in_date: "2015-04-09".to_string(),
            out_date: "2015-04-10".to_string(),
            room_type: RoomType {
                bookable_rate: 109.00,
                code: "KNG".to_string(),
                room_description: "King sized bed".to_string(),
                total_rate: 109.00,
                total_rate_inclusive: 123.17,
            },
        },
    ];

    for i in 7..=80 {
        if i % 3 != 0 {
            continue;
        }

        let hotel_id = i.to_string();

        let end_date = if i % 2 == 0 {
            "2015-04-17"
        } else {
            "2015-04-24"
        }
        .to_string();

        let (rate, rate_inc) = match i % 5 {
            1 => (120.00, 140.00),
            2 => (124.00, 144.00),
            3 => (132.00, 158.00),
            4 => (232.00, 258.00),
            _ => (109.00, 123.17),
        };

        new_rate_plans.push(RatePlan {
            hotel_id,
            code: "RACK".to_string(),
            in_date: "2015-04-09".to_string(),
            out_date: end_date,
            room_type: RoomType {
                bookable_rate: rate,
                code: "KNG".to_string(),
                room_description: "King sized bed".to_string(),
                total_rate: rate,
                total_rate_inclusive: rate_inc,
            },
        });
    }

    new_rate_plans
}

pub async fn initialize_database(url: &str) -> Result<Client, Box<dyn std::error::Error>> {
    log::info!("Attempting connection to {}", url);

    let client_options = ClientOptions::parse(&url).await?;
    let client = Client::with_options(client_options)?;
    log::info!("Successfully connected to MongoDB");

    let collection = client
        .database("rate-db")
        .collection::<RatePlan>("inventory");

    // empty collection first
    collection.delete_many(doc! {}, None).await?;

    let new_rate_plans = generate_test_data();
    collection.insert_many(&new_rate_plans, None).await?;
    log::info!("Successfully inserted test data into rate DB");

    // TODO: create index?

    Ok(client)
}

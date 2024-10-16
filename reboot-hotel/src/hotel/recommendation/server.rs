use ::recommendation::JsonParser;
use geo::point;
use geo::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tonic::{transport::Server, Request, Response, Status}; // Import geo functionality

use recommendation::recommendation_server::{Recommendation, RecommendationServer};
use recommendation::{RecommendationReply, RecommendationRequest};
pub mod hotel {
    pub mod recommendation {
        tonic::include_proto!("recommendation");
    }
}
#[derive(Debug, Default)]
pub struct MyRecommendation {
    hotels: HashMap<String, Hotel>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Hotel {
    #[serde(rename = "hotelId")]
    pub h_id: String,

    #[serde(rename = "lat")]
    pub h_lat: f64,

    #[serde(rename = "lon")]
    pub h_lon: f64,

    #[serde(rename = "rate")]
    pub h_rate: f64,

    #[serde(rename = "price")]
    pub h_price: f64,
}

#[tonic::async_trait]
impl Recommendation for MyRecommendation {
    async fn get_recommendations(
        &self,
        request: Request<RecommendationRequest>,
    ) -> Result<Response<RecommendationReply>, Status> {
        println!("Got a request: {:?}", request);
        let res = get_recommend_based_on_require(request, &self.hotels);
        println!("Reply is {:#?}", res);
        let reply = RecommendationReply { hotel_ids: res };
        Ok(Response::new(reply))
    }
}

/**
 * Returns a vector of hotel IDs based on a specified requirement (distance, rate, or price).
 *
 * This function takes a `Request` containing a `require` field that specifies whether the hotels
 * should be selected based on proximity ("dis"), rating ("rate"), or price ("price").
 *
 * It evaluates the requirement and processes the list of hotels accordingly, returning the
 * IDs of the hotels that meet the criteria. If the requirement is "dis", the function finds
 * the closest hotel(s) to the given latitude and longitude (specified in the request).
 * If the requirement is "rate", it returns the highest-rated hotel(s).
 * If the requirement is "price", it returns the cheapest hotel(s).
 *
 * # Arguments:
 * - `request`: A `Request` containing the requirement (distance, rate, or price) and the
 *              coordinates (latitude and longitude) for proximity-based recommendations.
 *
 * # Returns:
 * - A `Vec<String>` containing the hotel IDs that meet the requirement.
 *
 * # Example:
 *
 * let recommendations = get_recommend_based_on_require(request);
 * println!("Recommended hotels: {:?}", recommendations);
 *
 */
fn get_recommend_based_on_require(
    request: Request<RecommendationRequest>,
    hotels_map: &HashMap<String, Hotel>,
) -> Vec<String> {
    let mut res: Vec<String> = vec![];
    let req = request.into_inner(); // Only call `into_inner` once
    let require = req.require;
    let p1 = point!(x: req.lon, y: req.lat); // p1 is the reference point
    let mut min = f64::MAX;
    // Compare based on dis
    if require == "dis" {
        for (_key, hotel) in hotels_map {
            let current_point = point!(x: hotel.h_lon, y: hotel.h_lat);
            let tmp = p1.haversine_distance(&current_point) / 1000.0;
            if tmp < min {
                min = tmp;
            }
        }
        for (_key, hotel) in hotels_map {
            let current_point = point!(x: hotel.h_lon, y: hotel.h_lat);
            let tmp = p1.haversine_distance(&current_point) / 1000.0;
            if tmp == min {
                res.push(hotel.h_id.clone())
            }
        }
    } else if require == "rate" {
        let mut max = 0.0;
        for (_key, hotel) in hotels_map {
            if hotel.h_rate > max {
                max = hotel.h_rate
            }
        }
        for (_key, hotel) in hotels_map {
            if hotel.h_rate == max {
                res.push(hotel.h_id.clone())
            }
        }
    } else if require == "price" {
        let mut min = f64::MAX;
        for (_key, hotel) in hotels_map {
            if hotel.h_price < min {
                min = hotel.h_price
            }
        }
        for (_key, hotel) in hotels_map {
            if hotel.h_price == min {
                res.push(hotel.h_id.clone())
            }
        }
    }
    res
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;

    // read data from json
    let jp = JsonParser::new();
    let data = jp.read("./data/mock_data.json");
    let mut hotel_map: HashMap<String, Hotel> = HashMap::new();
    for item in data.as_array().unwrap() {
        // Accessing fields
        let hotel_id = item["hotelId"].as_str().unwrap(); // Access as string
        let lat = item["lat"].as_f64().unwrap(); // Access as f64 (float)
        let lon = item["lon"].as_f64().unwrap();
        let rate = item["rate"].as_f64().unwrap();
        let price = item["price"].as_f64().unwrap();
        // Insert into the HashMap
        hotel_map.insert(
            hotel_id.to_string(), // Use .to_string() to convert &str to String
            Hotel {
                h_id: hotel_id.to_string(),
                h_lat: lat,
                h_lon: lon,
                h_rate: rate,
                h_price: price,
            },
        );
    }

    let greeter = MyRecommendation {
        // moved ownership, no need to clone()
        hotels: hotel_map,
    };

    Server::builder()
        .add_service(RecommendationServer::new(greeter))
        .serve(addr)
        .await?;

    Ok(())
}

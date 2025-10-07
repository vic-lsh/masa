use geo::point;
use geo::prelude::*;
use std::collections::HashMap;
use tonic::{Request, Response, Status};

use crate::config::RecommendationConfig;
use crate::db;
pub mod hotel_tonic {
    pub mod recommendation {
        tonic::include_proto!("recommendation");
    }
}
use hotel_tonic::recommendation::recommendation_server::Recommendation;
use hotel_tonic::recommendation::{RecommendationReply, RecommendationRequest};

#[derive(Debug, Default)]
pub struct RecommendationImpl {
    hotels: HashMap<String, db::Hotel>,
}

impl RecommendationImpl {
    pub fn new(_config: RecommendationConfig) -> Self {
        let hotel_list = db::generate_test_data();
        let mut hotels = HashMap::new();
        for hotel in hotel_list {
            hotels.insert(hotel.hotel_id.clone(), hotel);
        }

        Self { hotels }
    }
}

#[tonic::async_trait]
impl Recommendation for RecommendationImpl {
    async fn get_recommendations(
        &self,
        request: Request<RecommendationRequest>,
    ) -> Result<Response<RecommendationReply>, Status> {
        println!("Got a request: {:?}", request);
        let res = get_recs(request, &self.hotels);
        println!("Reply is {:#?}", res);
        let reply = RecommendationReply { hotel_ids: res };
        Ok(Response::new(reply))
    }
}

fn get_recs(
    request: Request<RecommendationRequest>,
    hotels_map: &HashMap<String, db::Hotel>,
) -> Vec<String> {
    let mut res: Vec<String> = vec![];
    let req = request.into_inner();
    let require = req.require;
    let p1 = point!(x: req.lon, y: req.lat);
    let mut min = f64::MAX;
    if require == "dis" {
        for (_key, hotel) in hotels_map {
            let current_point = point!(x: hotel.lon, y: hotel.lat);
            let tmp = p1.haversine_distance(&current_point) / 1000.0;
            if tmp < min {
                min = tmp;
            }
        }
        for (_key, hotel) in hotels_map {
            let current_point = point!(x: hotel.lon, y: hotel.lat);
            let tmp = p1.haversine_distance(&current_point) / 1000.0;
            if tmp == min {
                res.push(hotel.hotel_id.clone())
            }
        }
    } else if require == "rate" {
        let mut max = 0.0;
        for (_key, hotel) in hotels_map {
            if hotel.rate > max {
                max = hotel.rate
            }
        }
        for (_key, hotel) in hotels_map {
            if hotel.rate == max {
                res.push(hotel.hotel_id.clone())
            }
        }
    } else if require == "price" {
        let mut min = f64::MAX;
        for (_key, hotel) in hotels_map {
            if hotel.price < min {
                min = hotel.price
            }
        }
        for (_key, hotel) in hotels_map {
            if hotel.price == min {
                res.push(hotel.hotel_id.clone())
            }
        }
    }
    res
}

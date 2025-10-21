pub mod hotel_tonic {
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
}
use app_utils::stats::AvgTracker;
use hotel_tonic::reservation::{self, reservation_server::Reservation};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use crate::config::ReservationConfig;
use crate::db;
use app_util_macros::track_latency;
use app_utils::stats::latency::StatsTracker;
use mongodb::{
    bson::{doc, Bson},
    Client as MongoClient, Collection,
};
use tonic::{Request, Response, Status};

pub struct ReservationImpl {
    mongo_client: MongoClient,
    mongo_reserve_client: MongoClient,
    check_avail_capacity_queries: Arc<AvgTracker>,
    check_avail_reservation_queries: Arc<AvgTracker>,
    mk_reserve_mongo: Arc<AvgTracker>,

    check_avail_stats: Arc<StatsTracker>,
}

impl ReservationImpl {
    pub async fn new(config: ReservationConfig) -> Result<Self, Box<dyn Error>> {
        let mongo_client = crate::db::initialize_database(&config.mongodb_addr).await?;
        let client_options = mongodb::options::ClientOptions::parse(&config.mongodb_addr).await?;
        let mongo_reserve_client = MongoClient::with_options(client_options)?;

        let check_avail_capacity_queries = Arc::new(AvgTracker::default());
        let check_avail_reservation_queries = Arc::new(AvgTracker::default());
        let mk_reserve_mongo = Arc::new(AvgTracker::default());

        Ok(Self {
            mongo_client,
            mongo_reserve_client,
            check_avail_capacity_queries,
            check_avail_reservation_queries,
            mk_reserve_mongo,

            check_avail_stats: Arc::new(StatsTracker::new(
                vec!["e2e", "mongo_get_capacity", "mongo_get_reservations"],
                true,
            )),
        })
    }
}

#[tonic::async_trait]
impl Reservation for ReservationImpl {
    async fn check_availability(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        use futures::StreamExt;

        let start = Instant::now();

        let req = req.into_inner();

        let mut res_map: HashMap<String, bool> = HashMap::new();
        for hotel_id in &req.hotel_ids {
            res_map.insert(hotel_id.clone(), true);
        }

        self.check_avail_capacity_queries.track(req.hotel_ids.len());

        let in_date_input = req.in_date.clone();
        let out_date_input = req.out_date.clone();

        let in_date = chrono::NaiveDate::parse_from_str(&in_date_input, "%Y-%m-%d")
            .map_err(|e| Status::invalid_argument(format!("invalid in_date: {}", e)))?;
        let out_date = chrono::NaiveDate::parse_from_str(&out_date_input, "%Y-%m-%d")
            .map_err(|e| Status::invalid_argument(format!("invalid out_date: {}", e)))?;

        let database = self.mongo_client.database("reservation-db");
        let num_collection = database.collection::<db::Number>("number");
        let reservation_collection = database.collection::<db::Reservation>("reservation");

        let cursor = num_collection
            .find(doc! { "hotelId": { "$in": &req.hotel_ids } }, None)
            .await
            .map_err(|e| Status::internal(format!("mongo error: {}", e)))?;

        let capacity_results = track_latency!(self.check_avail_stats.get("mongo_get_capacity"), {
            cursor.collect::<Vec<_>>().await
        });

        let mut capacity_map = HashMap::new();
        for result in capacity_results {
            if let Ok(num) = result {
                capacity_map.insert(num.hotel_id.clone(), num.number);
            }
        }

        for hotel_id in &req.hotel_ids {
            if !capacity_map.contains_key(hotel_id) {
                res_map.insert(hotel_id.clone(), false);
            }
        }

        let mut reservation_queries: usize = 0;

        let pipeline = vec![
            doc! {
                "$match": {
                    "hotelId": { "$in": &req.hotel_ids },
                    "inDate": { "$gte": &in_date_input, "$lt": &out_date_input },
                }
            },
            doc! {
                "$group": {
                    "_id": { "hotelId": "$hotelId", "inDate": "$inDate" },
                    "total_reserved": { "$sum": "$number" },
                }
            },
        ];

        let cursor = reservation_collection
            .aggregate(pipeline, None)
            .await
            .map_err(|e| Status::internal(format!("mongo error: {}", e)))?;

        let aggregation_results =
            track_latency!(self.check_avail_stats.get("mongo_get_reservations"), {
                cursor.collect::<Vec<_>>().await
            });

        let mut reserved_totals: HashMap<String, i32> = HashMap::new();
        for result in aggregation_results {
            if let Ok(doc) = result {
                if let Ok(id_doc) = doc.get_document("_id") {
                    if let (Some(Bson::String(hotel_id)), Some(Bson::String(in_date))) =
                        (id_doc.get("hotelId"), id_doc.get("inDate"))
                    {
                        let total_reserved = match doc.get("total_reserved") {
                            Some(Bson::Int32(v)) => *v,
                            Some(Bson::Int64(v)) => *v as i32,
                            Some(Bson::Double(v)) => *v as i32,
                            _ => 0,
                        };
                        let key = format!("{}::{}", hotel_id, in_date);
                        reserved_totals.insert(key, total_reserved);
                    }
                }
            }
        }

        for hotel_id in &req.hotel_ids {
            let Some(capacity) = capacity_map.get(hotel_id) else {
                continue;
            };

            let mut current_date = in_date;
            while current_date < out_date {
                let in_date_str = current_date.format("%Y-%m-%d").to_string();
                let next_date = current_date + chrono::Duration::days(1);
                current_date = next_date;

                reservation_queries += 1;

                let key = format!("{}::{}", hotel_id, in_date_str);
                let reserved_rooms = reserved_totals.get(&key).copied().unwrap_or(0);

                if reserved_rooms + req.room_number > *capacity {
                    res_map.insert(hotel_id.clone(), false);
                    break;
                }
            }
        }

        self.check_avail_reservation_queries
            .track(reservation_queries);

        // Collect results
        let mut resp = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };
        for (hotel_id, available) in res_map {
            if available {
                resp.hotel_ids.push(hotel_id);
            }
        }

        {
            let elapsed = start.elapsed().as_micros();
            self.check_avail_stats
                .get("e2e")
                .track(elapsed.try_into().unwrap());
        }

        Ok(Response::new(resp))
    }

    async fn make_reservation(
        &self,
        req: Request<reservation::ReservationRequest>,
    ) -> Result<Response<reservation::ReservationResponse>, Status> {
        let req = req.into_inner();

        let mut res = reservation::ReservationResponse {
            hotel_ids: Vec::new(),
        };

        let database = self.mongo_reserve_client.database("reservation-db");
        let res_collection: Collection<db::Reservation> = database.collection("reservation");
        let num_collection: Collection<db::Number> = database.collection("number");
        let in_date = chrono::NaiveDate::parse_from_str(&req.in_date, "%Y-%m-%d")
            .map_err(|e| Status::invalid_argument(format!("invalid in_date: {}", e)))?;
        let out_date = chrono::NaiveDate::parse_from_str(&req.out_date, "%Y-%m-%d")
            .map_err(|e| Status::invalid_argument(format!("invalid out_date: {}", e)))?;

        let hotel_id = &req.hotel_ids[0];

        let hotel_cap = num_collection
            .find_one(doc! { "hotelId": hotel_id }, None)
            .await
            .map_err(|e| Status::internal(format!("mongo error: {}", e)))?
            .ok_or_else(|| Status::not_found(format!("hotel {} not found", hotel_id)))?
            .number;

        let mut current_date = in_date;
        let mut iters = 0;

        use futures::StreamExt;

        while current_date < out_date {
            iters += 1;

            let in_date_str = current_date.format("%Y-%m-%d").to_string();
            let next_date = current_date + chrono::Duration::days(1);
            let out_date_str = next_date.format("%Y-%m-%d").to_string();

            let filter = doc! {
                "hotelId": hotel_id,
                "inDate": &in_date_str,
                "outDate": &out_date_str
            };

            let mut reservations_cursor = res_collection
                .find(filter, None)
                .await
                .map_err(|e| Status::internal(format!("mongo error: {}", e)))?;

            let mut reserved_rooms = 0;
            while let Some(reservation) = reservations_cursor.next().await {
                reserved_rooms += reservation
                    .map_err(|e| Status::internal(format!("mongo error: {}", e)))?
                    .number;
            }

            if reserved_rooms + req.room_number > hotel_cap {
                return Ok(Response::new(res));
            }

            current_date = next_date;
        }
        self.mk_reserve_mongo.track(iters);

        let mut reservations = Vec::new();
        let mut current_date = in_date;
        while current_date < out_date {
            let in_date_str = current_date.format("%Y-%m-%d").to_string();
            let next_date = current_date + chrono::Duration::days(1);
            let out_date_str = next_date.format("%Y-%m-%d").to_string();

            let reservation = db::Reservation {
                hotel_id: hotel_id.clone(),
                customer_name: req.customer_name.clone(),
                in_date: in_date_str,
                out_date: out_date_str,
                number: req.room_number,
            };
            reservations.push(reservation);

            current_date = next_date;
        }
        res_collection
            .insert_many(reservations, None)
            .await
            .map_err(|e| Status::internal(format!("mongo error: {}", e)))?;
        res.hotel_ids.push(hotel_id.clone());

        // {
        //     let elapsed = start.elapsed().as_micros();
        //     self.lat_make_reserve.lock().unwrap().track(elapsed as u64);
        // }

        Ok(Response::new(res))
    }
}

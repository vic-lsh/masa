pub mod hotel_tonic {
    pub mod geo {
        tonic::include_proto!("geo");
    }
}

use app_utils::stats::latency::{new_latency_tracker, spawn_latency_logger, SyncLatencyTracker};
use kiddo::KdTree;
use kiddo::SquaredEuclidean;
use std::time::Duration;
use tonic::{Request, Response, Status};

use hotel_tonic::{geo, geo::geo_server::Geo};

use crate::config::GeoConfig;
use crate::db;

struct GeoIndex {
    tree: KdTree<f64, 2>,
    points: Vec<db::Point>,
}

impl GeoIndex {
    fn new() -> Self {
        GeoIndex {
            tree: KdTree::new(),
            points: Vec::new(),
        }
    }

    fn add_point(&mut self, point: db::Point) {
        self.tree.add(
            &[point.lat, point.lon],
            self.points.len().try_into().unwrap(),
        );
        self.points.push(point);
    }

    fn find_nearest(&self, query_lat: f64, query_lon: f64, k: usize) -> Vec<(&db::Point, f64)> {
        let query_point = [query_lat, query_lon];

        let nearest = self.tree.nearest_n::<SquaredEuclidean>(&query_point, k);

        nearest
            .into_iter()
            .map(|p| (&self.points[p.item as usize], p.distance.sqrt()))
            .collect()
    }
}

pub struct GeoImpl {
    index: GeoIndex,
    latency_tracker: SyncLatencyTracker,
}

impl GeoImpl {
    pub fn new(_config: GeoConfig) -> Self {
        let (latency_tracker, latency_consumer) = new_latency_tracker("GeoSvc");
        spawn_latency_logger(latency_consumer, Duration::from_secs(30));

        let points = db::generate_test_data();
        let mut index = GeoIndex::new();
        for p in points {
            index.add_point(p);
        }
        GeoImpl {
            index,
            latency_tracker,
        }
    }
}

#[tonic::async_trait]
impl Geo for GeoImpl {
    async fn get_nearby(
        &self,
        request: Request<geo::NearbyRequest>,
    ) -> Result<Response<geo::NearbyResponse>, Status> {
        let start = std::time::Instant::now();
        const MAX_SEARCH_RESULTS: usize = 5;

        let request = request.into_inner();
        let result = self
            .index
            .find_nearest(request.lat, request.lon, MAX_SEARCH_RESULTS);

        let hotel_ids = result.into_iter().map(|r| r.0.pid.to_owned()).collect();
        let response = geo::NearbyResponse { hotel_ids };
        log::info!("response: {:?}", response);
        self.latency_tracker
            .track(start.elapsed().as_micros().try_into().unwrap());
        Ok(Response::new(response))
    }
}

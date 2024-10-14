use std::collections::HashMap;
use std::sync::OnceLock;

use tonic_masa::{GlobalGraph, LocalGraph, MethodId, ServiceId, Span};

static TRACKER_CAPACITY: Option<usize> = Some(100);
// static TRACKER_CAPACITY: Option<usize> = Some(1_000);
static GLOBAL_GRAPHS: OnceLock<HashMap<ServiceId, GlobalGraph>> = OnceLock::new();

fn get_frontend() -> GlobalGraph {
    let service_id: ServiceId = "frontend.Frontend".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/frontend.Frontend/HandleSearch".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(
                service_id.clone(),
                method_id.clone(),
                vec![
                    Span::new(
                        "/search.Search/HandleNearby".to_string() as MethodId,
                        None,
                        TRACKER_CAPACITY,
                    ),
                    Span::new(
                        "/profile.Profile/HandleGetProfiles".to_string() as MethodId,
                        None,
                        TRACKER_CAPACITY,
                    ),
                ],
            ),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(service_id, local_graphs);
    global_graph
}

fn get_search() -> GlobalGraph {
    let service_id: ServiceId = "search.Search".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/search.Search/HandleNearby".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(
                service_id.clone(),
                method_id.clone(),
                vec![
                    Span::new(
                        "/geo.Geo/HandleNearby".to_string() as MethodId,
                        None,
                        TRACKER_CAPACITY,
                    ),
                    Span::new(
                        "/rate.Rate/HandleGetRates".to_string() as MethodId,
                        None,
                        TRACKER_CAPACITY,
                    ),
                ],
            ),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(service_id, local_graphs);
    global_graph
}

fn get_geo() -> GlobalGraph {
    let service_id: ServiceId = "geo.Geo".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/geo.Geo/HandleNearby".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(service_id.clone(), method_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(service_id, local_graphs);
    global_graph
}

fn get_rate() -> GlobalGraph {
    let service_id: ServiceId = "rate.Rate".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/rate.Rate/HandleGetRates".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(service_id.clone(), method_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(service_id, local_graphs);
    global_graph
}

fn get_profile() -> GlobalGraph {
    let service_id: ServiceId = "profile.Profile".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/profile.Profile/HandleGetProfiles".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(service_id.clone(), method_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(service_id, local_graphs);
    global_graph
}

fn get_global_graphs() -> &'static HashMap<ServiceId, GlobalGraph> {
    GLOBAL_GRAPHS.get_or_init(|| {
        let mut m = HashMap::new();

        let frontend = get_frontend();
        m.insert(frontend.service_id().clone(), frontend);

        let search = get_search();
        m.insert(search.service_id().clone(), search);

        let geo = get_geo();
        m.insert(geo.service_id().clone(), geo);

        let rate = get_rate();
        m.insert(rate.service_id().clone(), rate);

        let profile = get_profile();
        m.insert(profile.service_id().clone(), profile);

        m
    })
}

/// Get a global graph of Hotel Reservation.
#[allow(dead_code)]
pub(crate) fn get_global_graph(service_id: ServiceId) -> GlobalGraph {
    log::info!("get_global_graph, service_id: {:?}", service_id);
    let global_graphs = get_global_graphs();
    global_graphs[&service_id].clone()
}

use std::collections::HashMap;
use std::sync::OnceLock;

use tonic_masa::{GlobalGraph, LocalGraph, MethodId, ServiceId, Span};

static TRACKER_CAPACITY: Option<usize> = Some(100);
static GLOBAL_GRAPHS: OnceLock<HashMap<ServiceId, GlobalGraph>> = OnceLock::new();

fn get_global_graph_frontend() -> GlobalGraph {
    let service_id: ServiceId = "/hotel.Frontend".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/hotel.Frontend/SayFrontend".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(
                service_id.clone(),
                method_id.clone(),
                vec![
                    Span::new(
                        "/hotel.Search/SaySearch".to_string() as MethodId,
                        None,
                        TRACKER_CAPACITY,
                    ),
                    Span::new(
                        "/hotel.Profile/SayProfile".to_string() as MethodId,
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

fn get_global_graph_search() -> GlobalGraph {
    let service_id: ServiceId = "/hotel.Search".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/hotel.Search/SaySearch".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(service_id.clone(), method_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(service_id, local_graphs);
    global_graph
}

fn get_global_graph_profile() -> GlobalGraph {
    let service_id: ServiceId = "/hotel.Profile".to_string();
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/hotel.Profile/SayProfile".to_string();
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
        let frontend = get_global_graph_frontend();
        m.insert(frontend.service_id().clone(), frontend);
        let search = get_global_graph_search();
        m.insert(search.service_id().clone(), search);
        let profile = get_global_graph_profile();
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

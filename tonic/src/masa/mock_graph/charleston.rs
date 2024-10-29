use std::collections::HashMap;
use tonic_masa::{
    Distribution as MasaDistribution, GlobalGraph, GraphId, LocalGraph, MethodId, Span,
};

/// Get a global graph with two hops.
#[allow(dead_code)]
pub(crate) fn get_global_graph_i2(
    graph_id: GraphId,
    tracker_capacity: Option<usize>,
    mean: u64,
) -> GlobalGraph {
    let local_graphs = {
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/hello.Greeter/SayHello";
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(
                graph_id.clone(),
                method_id.clone(),
                vec![Span::new(
                    "/hello.Greeter/SayGoodbye".to_string(),
                    Some(MasaDistribution::new(mean, Vec::new())),
                    tracker_capacity,
                )],
            ),
        );
        let method_id: MethodId = "/hello.Greeter/SayGoodbye";
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(graph_id.clone(), method_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(graph_id, local_graphs);
    global_graph
}

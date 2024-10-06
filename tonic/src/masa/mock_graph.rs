use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp};
use std::collections::HashMap;
use tonic_masa::{
    Distribution as MasaDistribution, GlobalGraph, GraphId, Latency, LocalGraph, MethodId, Span,
};

/// Get a global graph with two hops.
#[allow(dead_code)]
pub fn get_global_graph_i2(
    graph_id: GraphId,
    tracker_capacity: Option<usize>,
    mean: u64,
) -> GlobalGraph {
    let local_graphs = {
        let mut _rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        let method_id: MethodId = "/hello.Greeter/SayHello".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(
                graph_id.clone(),
                method_id.clone(),
                vec![Span::new(
                    "/hello.Greeter/SayGoodbye".to_string() as MethodId,
                    Some(MasaDistribution::new(mean, Vec::new())),
                    tracker_capacity,
                )],
            ),
        );
        let method_id: MethodId = "/hello.Greeter/SayGoodbye".to_string();
        graphs.insert(
            method_id.clone(),
            LocalGraph::new(graph_id.clone(), method_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(graph_id, local_graphs);
    global_graph
}

fn get_percentile_latencies(
    rng: &mut StdRng,
    mean: Latency,
    n_samples: usize,
    n_percentiles: usize,
) -> Vec<Latency> {
    let exp = Exp::new(1.0 / mean as f64).unwrap();
    let mut samples = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        samples.push(exp.sample(rng) as Latency);
    }
    let mut percentiles = Vec::with_capacity(n_percentiles);
    samples.sort();
    for i in 0..n_percentiles {
        let index = (i * n_samples) / n_percentiles;
        percentiles.push(samples[index]);
    }
    percentiles
}

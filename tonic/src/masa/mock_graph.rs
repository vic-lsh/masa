use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp};
use std::collections::HashMap;
use tonic_masa::{Distribution as MasaDistribution, GlobalGraph, Latency, LocalGraph, Path, Span};

/// Get a global graph with two hops.
#[allow(dead_code)]
pub fn get_global_graph_i2(
    graph_id: Path, // GraphId
    tracker_capacity: Option<usize>,
    mean: u64,
) -> GlobalGraph {
    let local_graphs = {
        let mut _rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        graphs.insert(
            "/hello.Greeter/SayHello".to_string() as Path, // MethodId
            LocalGraph::new(
                // {GraphId, Vec<Span<MethodId>>}
                graph_id.clone(), // GraphId
                vec![Span::new(
                    "/hello.Greeter/SayGoodbye".to_string(), // MethodId
                    Some(MasaDistribution::new(mean, Vec::new())),
                    tracker_capacity,
                )],
            ),
        );
        graphs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as Path, // MethodId
            LocalGraph::new(graph_id.clone(), vec![]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new(
        graph_id,     // GraphId
        local_graphs, // HashMap<MethodId, LocalGraph>
    );
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

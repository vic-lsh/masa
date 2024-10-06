use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp};
use std::collections::HashMap;
use tonic_masa::{Distribution as MasaDistribution, GlobalGraph, Latency, LocalGraph, Path, Span};

#[allow(dead_code)]
pub fn get_global_graph_i2(
    graph_id: Path,
    n_samples: usize,
    n_percentiles: usize,
    tracker_capacity: Option<usize>,
    mean: u64,
) -> GlobalGraph {
    let local_graphs = {
        let mut rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        graphs.insert(
            "/hello.Greeter/SayHello".to_string() as Path,
            LocalGraph::new(
                graph_id.clone(),
                vec![
                    Span::new(
                        "/hello.Greeter/SayHello/Head".to_string(),
                        Some(MasaDistribution::new(
                            mean / 2,
                            get_percentile_latencies(&mut rng, mean / 2, n_samples, n_percentiles),
                        )),
                        tracker_capacity,
                    ),
                    Span::new(
                        "/hello.Greeter/SayGoodbye".to_string(),
                        Some(MasaDistribution::new(mean, Vec::new())),
                        tracker_capacity,
                    ),
                    Span::new(
                        "/hello.Greeter/SayHello/Tail".to_string(),
                        Some(MasaDistribution::new(
                            mean / 2,
                            get_percentile_latencies(&mut rng, mean / 2, n_samples, n_percentiles),
                        )),
                        tracker_capacity,
                    ),
                ],
            ),
        );
        graphs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as Path,
            LocalGraph::new(
                graph_id.clone(),
                vec![
                    Span::new(
                        "/hello.Greeter/SayGoodbye/Head".to_string(),
                        Some(MasaDistribution::new(
                            mean / 2,
                            get_percentile_latencies(&mut rng, mean / 2, n_samples, n_percentiles),
                        )),
                        tracker_capacity,
                    ),
                    Span::new(
                        "/hello.Greeter/SayGoodbye/Tail".to_string(),
                        Some(MasaDistribution::new(
                            mean / 2,
                            get_percentile_latencies(&mut rng, mean / 2, n_samples, n_percentiles),
                        )),
                        tracker_capacity,
                    ),
                ],
            ),
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

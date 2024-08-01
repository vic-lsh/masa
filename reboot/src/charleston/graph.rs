use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp};
use std::collections::HashMap;
use tonic::metadata::{
    Distribution as MasaDistribution, GlobalGraph, Latency, LocalGraph, Path, Span,
};

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

pub fn get_global_graph() -> GlobalGraph {
    let local_graphs = {
        let mut rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        let n_samples = 1_000;
        let n_percentiles = 1_000;
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraph::new(vec![Span::new(
                "/hello.Greeter/SayHello".to_string(),
                MasaDistribution::new(1_000, None),
            )]),
        );
        graphs.insert(
            "/hello.Greeter/SayHello".to_string() as Path,
            LocalGraph::new(vec![
                Span::new(
                    "Head".to_string(),
                    MasaDistribution::new(
                        1_000,
                        Some(get_percentile_latencies(
                            &mut rng,
                            1_000,
                            n_samples,
                            n_percentiles,
                        )),
                    ),
                ),
                Span::new(
                    "/hello.Greeter/SayGoodbye".to_string(),
                    MasaDistribution::new(2_000, None),
                ),
                Span::new(
                    "Tail".to_string(),
                    MasaDistribution::new(
                        3,
                        Some(get_percentile_latencies(
                            &mut rng,
                            3,
                            n_samples,
                            n_percentiles,
                        )),
                    ),
                ),
            ]),
        );
        graphs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as Path,
            LocalGraph::new(vec![
                Span::new(
                    "Head".to_string(),
                    MasaDistribution::new(
                        2_000,
                        Some(get_percentile_latencies(
                            &mut rng,
                            2_000,
                            n_samples,
                            n_percentiles,
                        )),
                    ),
                ),
                Span::new(
                    "Tail".to_string(),
                    MasaDistribution::new(
                        4,
                        Some(get_percentile_latencies(
                            &mut rng,
                            4,
                            n_samples,
                            n_percentiles,
                        )),
                    ),
                ),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new("GID".to_string() as Path, local_graphs);
    global_graph
}

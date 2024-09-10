use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, Exp};
use std::collections::HashMap;
use tonic_masa::{
    Distribution as MasaDistribution, GlobalGraphInner, Latency, LocalGraphInner, Path, SpanInner,
};

// [TODO] Support Hotel.
#[allow(dead_code)]
pub fn get_global_graph_hotel() -> GlobalGraphInner {
    panic!("Not implemented");
}

#[allow(dead_code)]
pub fn get_global_graph_tung_chung() -> GlobalGraphInner {
    let local_graphs = {
        let mut rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        let n_samples = 1_000;
        let n_percentiles = 1_000;
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraphInner::new(vec![SpanInner::new(
                "/bridge.TungChung/SayFrontend".to_string(),
                None,
                None,
            )]),
        );
        graphs.insert(
            "/bridge.TungChung/SayFrontend".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new("/bridge.TungChung/SaySearch".to_string(), None, None),
                // SpanInner::new(
                //     "/bridge.TungChung/SayReserve".to_string(),
                //     None,,
                // ),
                SpanInner::new("/bridge.TungChung/SayProfile".to_string(), None, None),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.TungChung/SaySearch".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new("/bridge.TungChung/SayGeo".to_string(), None, None),
                SpanInner::new("/bridge.TungChung/SayRate".to_string(), None, None),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.TungChung/SayGeo".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.TungChung/SayRate".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
                // Some(SpanInner::new(
                //     "Memcached".to_string(),
                //     ...,
                // )),
                // Some(SpanInner::new(
                //     "Mongodb".to_string(),
                //     ...,
                // )),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.TungChung/SayProfile".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
                // SpanInner::new(
                //     "Memcached".to_string(),
                //     ...,
                // ),
                // SpanInner::new(
                //     "Mongodb".to_string(),
                //     ...,
                // ),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraphInner::new("TungChung".to_string() as Path, local_graphs);
    global_graph
}

#[allow(dead_code)]
pub fn get_global_graph_i4() -> GlobalGraphInner {
    let local_graphs = {
        let mut rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        let n_samples = 1_000;
        let n_percentiles = 1_000;
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraphInner::new(vec![SpanInner::new(
                "/bridge.Worker/SayHelloI4".to_string(),
                None,
                None,
            )]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI4".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new("/bridge.Worker/SayHelloI3".to_string(), None, None),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        4,
                        get_percentile_latencies(&mut rng, 4, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI3".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new("/bridge.Worker/SayHelloI2".to_string(), None, None),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        3,
                        get_percentile_latencies(&mut rng, 3, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI2".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new("/bridge.Worker/SayHelloI1".to_string(), None, None),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        2,
                        get_percentile_latencies(&mut rng, 2, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI1".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraphInner::new("I4".to_string() as Path, local_graphs);
    global_graph
}

#[allow(dead_code)]
pub fn get_global_graph_i2() -> GlobalGraphInner {
    let local_graphs = {
        let mut rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        let n_samples = 1_000;
        let n_percentiles = 1_000;
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraphInner::new(vec![SpanInner::new(
                "/bridge.Worker/SayHelloI2".to_string(),
                None,
                None,
            )]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI2".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new("/bridge.Worker/SayHelloI1".to_string(), None, None),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        2,
                        get_percentile_latencies(&mut rng, 2, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI1".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraphInner::new("I2".to_string() as Path, local_graphs);
    global_graph
}

#[allow(dead_code)]
pub fn get_global_graph_i1() -> GlobalGraphInner {
    let local_graphs = {
        let mut rng = StdRng::seed_from_u64(998244353);
        let mut graphs = HashMap::new();
        let n_samples = 1_000;
        let n_percentiles = 1_000;
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraphInner::new(vec![SpanInner::new(
                "/bridge.Worker/SayHelloI1".to_string(),
                None,
                None,
            )]),
        );
        graphs.insert(
            "/bridge.Worker/SayHelloI1".to_string() as Path,
            LocalGraphInner::new(vec![
                SpanInner::new(
                    "Head".to_string(),
                    Some(MasaDistribution::new(
                        16_000,
                        get_percentile_latencies(&mut rng, 16_000, n_samples, n_percentiles),
                    )),
                    None,
                ),
                SpanInner::new(
                    "Tail".to_string(),
                    Some(MasaDistribution::new(
                        1,
                        get_percentile_latencies(&mut rng, 1, n_samples, n_percentiles),
                    )),
                    None,
                ),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraphInner::new("I1".to_string() as Path, local_graphs);
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

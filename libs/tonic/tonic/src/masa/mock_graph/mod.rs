use rand::rngs::StdRng;
use rand_distr::{Distribution, Exp};
use masa::Latency;

pub(crate) mod charleston;
pub(crate) mod hotel;

#[allow(dead_code)]
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

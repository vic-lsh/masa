use rand::{thread_rng, Rng};

/// Sample whether a call should be made based on probability.
/// Returns true if a random value [0.0, 1.0) is less than the given probability.
pub fn should_make_call(probability: f64) -> bool {
    let mut rng = thread_rng();
    rng.gen::<f64>() < probability
}

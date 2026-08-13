//! Tokio runtime construction shared across service binaries.

fn parse_worker_threads(value: &str) -> Option<usize> {
    let cpus = value.parse::<f64>().ok()?;
    if !cpus.is_finite() || cpus <= 0.0 {
        return None;
    }
    Some(cpus.ceil() as usize)
}

/// Number of worker threads for the multi-threaded runtime.
///
/// Reads `CPUS_PER_REPLICA` — the per-container CPU budget the experiment
/// harness also passes to Docker's `--cpus` — so the worker count matches the
/// available CFS budget. The default `num_cpus()` used by `#[tokio::main]`
/// reads the CPU affinity mask (the whole cpuset), which ignores the quota and
/// would spawn far more workers than there is budget for, oversubscribing it.
///
/// Falls back to [`std::thread::available_parallelism`] when the variable is
/// unset, such as when running a binary directly outside the harness.
fn worker_threads() -> usize {
    std::env::var("CPUS_PER_REPLICA")
        .ok()
        .as_deref()
        .and_then(parse_worker_threads)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        })
}

/// Runs a future on a multi-threaded Tokio runtime sized to the container's CPU
/// budget.
///
/// The `sched_mt` feature selects the shared priority scheduler inside Tokio.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads())
        .enable_all()
        .build()
        .expect("failed to build multi-thread tokio runtime")
        .block_on(future)
}

#[cfg(test)]
mod tests {
    use super::parse_worker_threads;

    #[test]
    fn parses_integer_and_fractional_cpu_quotas() {
        assert_eq!(parse_worker_threads("16"), Some(16));
        assert_eq!(parse_worker_threads("1.25"), Some(2));
        assert_eq!(parse_worker_threads("0.5"), Some(1));
    }

    #[test]
    fn rejects_invalid_cpu_quotas() {
        for value in ["", "invalid", "0", "-1", "NaN", "inf"] {
            assert_eq!(parse_worker_threads(value), None, "value: {value}");
        }
    }
}

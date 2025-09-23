use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

mod service {
    tonic::include_proto!("service");
}
use service::service_client::ServiceClient;
use service::RootRequest;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Read env vars (with defaults where sensible)
    let addr = env::var("IP").unwrap_or_else(|_| "[::1]".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "50051".to_string());
    let rps: f64 = env::var("RPS")
        .unwrap_or_else(|_| "100".to_string())
        .parse()?;
    let max_in_flight: usize = env::var("MAX_IN_FLIGHT")
        .unwrap_or_else(|_| "10000".to_string())
        .parse()?;
    let stats_interval_sec: u64 = env::var("STATS_INTERVAL_SEC")
        .unwrap_or_else(|_| "2".to_string())
        .parse()?;
    let hc_timeout_sec: u64 = env::var("HEALTHCHECK_TIMEOUT_SEC")
        .unwrap_or_else(|_| "60".to_string())
        .parse()?;
    let hc_backoff_ms: u64 = env::var("HEALTHCHECK_BACKOFF_MS")
        .unwrap_or_else(|_| "1000".to_string())
        .parse()?;

    if rps <= 0.0 {
        anyhow::bail!("RPS must be > 0");
    }
    let per_req = Duration::from_secs_f64(1.0 / rps);

    let addr = format!("http://{}:{}", addr, port);

    let channel = health_check_connect_and_call(
        &addr,
        Duration::from_secs(hc_timeout_sec),
        Duration::from_millis(hc_backoff_ms),
    )
    .await?;

    let client = ServiceClient::new(channel);

    // Counters
    let sent = Arc::new(AtomicU64::new(0));
    let ok = Arc::new(AtomicU64::new(0));
    let err = Arc::new(AtomicU64::new(0));
    let inflight_guard = Arc::new(Semaphore::new(max_in_flight));

    // Stats printer
    {
        let sent = sent.clone();
        let ok = ok.clone();
        let err = err.clone();
        tokio::spawn(async move {
            let mut last_sent = 0u64;
            let mut last_ok = 0u64;
            let mut last_err = 0u64;
            let mut ticker = tokio::time::interval(Duration::from_secs(stats_interval_sec));
            loop {
                ticker.tick().await;
                let s = sent.load(Ordering::Relaxed);
                let o = ok.load(Ordering::Relaxed);
                let e = err.load(Ordering::Relaxed);
                println!(
                    "[stats] sent={} (+{}), ok={} (+{}), err={} (+{})",
                    s,
                    s - last_sent,
                    o,
                    o - last_ok,
                    e,
                    e - last_err
                );
                last_sent = s;
                last_ok = o;
                last_err = e;
            }
        });
    }

    // Open-loop ticker
    let mut ticker = tokio::time::interval(per_req);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.reset();

    println!(
        "Starting loadgen: addr={}, rps={}, max_in_flight={}",
        addr, rps, max_in_flight
    );
    println!("Press Ctrl-C to stop.");

    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("ctrl-c handler failed");
    };
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                println!("Received Ctrl-C. Shutting down...");
                break;
            }
            _ = ticker.tick() => {
                let permit = match inflight_guard.clone().try_acquire_owned() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let client = client.clone();
                let sent = sent.clone();
                let ok = ok.clone();
                let err = err.clone();

                sent.fetch_add(1, Ordering::Relaxed);

                let mut rpc_client = client.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let req = Request::new(RootRequest {});

                    let _t0 = Instant::now();
                    let res = rpc_client.root(req).await;

                    match res {
                        Ok(_) => ok.fetch_add(1, Ordering::Relaxed),
                        Err(_) => err.fetch_add(1, Ordering::Relaxed),
                    };
                });
            }
        }
    }

    // Drain in-flight requests before exit
    let _ = inflight_guard.acquire_many(max_in_flight as u32).await;

    let s = sent.load(Ordering::Relaxed);
    let o = ok.load(Ordering::Relaxed);
    let e = err.load(Ordering::Relaxed);
    println!("Final stats: sent={}, ok={}, err={}", s, o, e);

    Ok(())
}

/// Try to connect and successfully invoke `root()` within `timeout`.
/// Retries every `backoff` until both (1) connect and (2) RPC response succeed.
async fn health_check_connect_and_call(
    addr: &str,
    timeout: Duration,
    backoff: Duration,
) -> anyhow::Result<Channel> {
    println!(
        "Health check: ensuring connectivity and root() response (timeout={}s)...",
        timeout.as_secs()
    );

    // Build a reusable Endpoint once (Clone is cheap)
    let endpoint = Endpoint::from_shared(addr.to_string())?.tcp_nodelay(true);

    let deadline = Instant::now() + timeout;
    loop {
        // (1) Connect
        match endpoint.clone().connect().await {
            Ok(ch) => {
                // (2) Issue a root() call and expect a response
                let mut client = ServiceClient::new(ch.clone());
                match client.root(Request::new(RootRequest {})).await {
                    Ok(_) => {
                        println!("Health check passed: connected and root() responded.");
                        return Ok(ch);
                    }
                    Err(e) => {
                        eprintln!("Health check: root() RPC failed: {e}");
                        if Instant::now() >= deadline {
                            anyhow::bail!(
                                "Health check failed: RPC did not succeed before timeout"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Health check: connect to {} failed: {e}", addr);
                if Instant::now() >= deadline {
                    anyhow::bail!("Health check failed: could not connect before timeout");
                }
            }
        }

        tokio::time::sleep(backoff).await;
    }
}

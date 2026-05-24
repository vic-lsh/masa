use crate::config::ParsedCall;
use crate::hop_trace::decode_hop_traces;
use crate::service_registry::ServiceRegistry;
use app_utils::timing::time_now;
use masa::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
use rand::{thread_rng, Rng};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tonic::metadata::MetadataValue;
use tonic::{Request, Status};
use tracing::warn;

static DISABLE_CPU_YIELD: LazyLock<bool> = LazyLock::new(|| {
    std::env::var("SYNTHBENCH_DISABLE_CPU_YIELD")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
});

/// Sample whether a call should be made based on probability.
/// Returns true if a random value [0.0, 1.0) is less than the given probability.
pub fn should_make_call(probability: f64) -> bool {
    let mut rng = thread_rng();
    rng.gen::<f64>() < probability
}

/// Simulates work by sleeping and busy-spinning.
///
/// * `duration_us` - The total duration of work in microseconds.
/// * `busy_spin_ratio` - The fraction of time to busy-spin [0.0, 1.0].
pub async fn simulate_work(duration_us: u64, busy_spin_ratio: f64) {
    if duration_us == 0 {
        return;
    }

    let busy_spin_dur_us = (duration_us as f64 * busy_spin_ratio).round() as u64;
    let sleep_dur_us = duration_us.saturating_sub(busy_spin_dur_us);

    // Sleep first
    if sleep_dur_us > 0 {
        tokio::time::sleep(Duration::from_micros(sleep_dur_us)).await;
    }

    // Then busy spin if needed
    if busy_spin_dur_us > 0 {
        if *DISABLE_CPU_YIELD {
            busy_spin(Duration::from_micros(busy_spin_dur_us));
            return;
        }

        let yield_interval = Duration::from_micros(200);
        let busy_spin_duration = Duration::from_micros(busy_spin_dur_us);

        let mut remaining = busy_spin_duration;
        while remaining > yield_interval {
            busy_spin(yield_interval);
            tokio::task::yield_now().await;
            remaining -= yield_interval;
        }
        if remaining > Duration::ZERO {
            busy_spin(remaining);
        }
    }
}

fn busy_spin(duration: Duration) {
    let end = Instant::now() + duration;

    while Instant::now() < end {}
}

pub async fn execute_call_sequence(
    registry: &ServiceRegistry,
    call_sequence: &[Vec<ParsedCall>],
) -> Result<Vec<crate::hop_trace::HopTrace>, Status> {
    let mut traces = Vec::new();

    // Execute steps sequentially
    for step in call_sequence {
        // Structured fanout: each step owns its task set and joins it before proceeding.
        let mut tasks = JoinSet::new();

        for call in step {
            if should_make_call(call.probability) {
                let client = registry.get_client_clone(&call.target.service_id).await;

                let client = match client {
                    Some(client) => client,
                    None => {
                        warn!(
                            "Service '{}' not yet connected, skipping call",
                            call.target.service_id
                        );
                        continue;
                    }
                };

                let sent_at = time_now();

                // Spawn task to make the call
                let target_service_id = call.target.service_id.clone();
                let target_method_name = call.target.method_name.clone();
                tasks.spawn(async move {
                    // Create metadata values first to avoid cloning strings
                    let method_meta = MetadataValue::try_from(target_method_name.as_str())
                        .map_err(|e| {
                            Status::internal(format!(
                                "Failed to create method name override: {:?}",
                                e
                            ))
                        })?;
                    let service_meta = MetadataValue::try_from(target_service_id.as_str())
                        .map_err(|e| {
                            Status::internal(format!(
                                "Failed to create service name override: {:?}",
                                e
                            ))
                        })?;

                    let mut request = Request::new(crate::tonic::child::MethodRequest {
                        service_id: target_service_id.clone(),
                        method_name: target_method_name.clone(),
                        sent_at,
                    });

                    request
                        .metadata_mut()
                        .insert(METHOD_NAME_OVERRIDE_HEADER, method_meta);
                    request
                        .metadata_mut()
                        .insert(SERVICE_NAME_OVERRIDE_HEADER, service_meta);

                    client.clone().handle_method(request).await
                });
            }
        }

        // Wait for all calls in this step; dropping the set aborts unfinished tasks on error.
        while let Some(task_result) = tasks.join_next().await {
            let rpc_result =
                task_result.map_err(|e| Status::internal(format!("Task join error: {}", e)))?;
            let response = rpc_result?;
            traces.extend(decode_hop_traces(&response.into_inner().hop_trace_json)?);
        }
    }

    Ok(traces)
}

#[cfg(feature = "sched_oracle")]
pub async fn execute_oracle_call_sequence(
    registry: &ServiceRegistry,
    child_steps: &[Vec<crate::oracle::PlannedCall>],
) -> Result<Vec<crate::hop_trace::HopTrace>, Status> {
    let mut traces = Vec::new();

    for step in child_steps {
        let mut tasks = JoinSet::new();

        for planned_call in step {
            let client = registry
                .get_client_clone(&planned_call.target.service_id)
                .await;

            let client = match client {
                Some(client) => client,
                None => {
                    warn!(
                        "Service '{}' not yet connected, skipping oracle-planned call",
                        planned_call.target.service_id
                    );
                    continue;
                }
            };

            let sent_at = time_now();
            let planned_call = planned_call.clone();

            tasks.spawn(async move {
                let target_service_id = planned_call.target.service_id.clone();
                let target_method_name = planned_call.target.method_name.clone();

                let method_meta =
                    MetadataValue::try_from(target_method_name.as_str()).map_err(|e| {
                        Status::internal(format!("Failed to create method name override: {:?}", e))
                    })?;
                let service_meta =
                    MetadataValue::try_from(target_service_id.as_str()).map_err(|e| {
                        Status::internal(format!("Failed to create service name override: {:?}", e))
                    })?;

                let mut request = Request::new(crate::tonic::child::MethodRequest {
                    service_id: target_service_id.clone(),
                    method_name: target_method_name.clone(),
                    sent_at,
                });

                request
                    .metadata_mut()
                    .insert(METHOD_NAME_OVERRIDE_HEADER, method_meta);
                request
                    .metadata_mut()
                    .insert(SERVICE_NAME_OVERRIDE_HEADER, service_meta);
                planned_call.inject_headers(request.metadata_mut())?;

                client.clone().handle_method(request).await
            });
        }

        while let Some(task_result) = tasks.join_next().await {
            let rpc_result =
                task_result.map_err(|e| Status::internal(format!("Task join error: {}", e)))?;
            let response = rpc_result?;
            traces.extend(decode_hop_traces(&response.into_inner().hop_trace_json)?);
        }
    }

    Ok(traces)
}

use crate::config::CallTarget;
use crate::service_registry::ServiceRegistry;
use app_utils::timing::time_now;
use rand::{thread_rng, Rng};
use std::time::{Duration, Instant};
use tonic::masa::{METHOD_NAME_OVERRIDE_HEADER, SERVICE_NAME_OVERRIDE_HEADER};
use tonic::metadata::MetadataValue;
use tonic::{Request, Status};
use tracing::warn;

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
    call_sequence: &[Vec<(CallTarget, f64)>],
) -> Result<(), Status> {
    // Execute steps sequentially
    for step in call_sequence {
        // Collect tasks for parallel calls in this step
        let mut tasks = Vec::new();

        for (target, probability) in step {
            if should_make_call(*probability) {
                let client = registry.get_client_clone(&target.service_id).await;

                let client = match client {
                    Some(client) => client,
                    None => {
                        warn!(
                            "Service '{}' not yet connected, skipping call",
                            target.service_id
                        );
                        continue;
                    }
                };

                let sent_at = time_now();

                // Spawn task to make the call
                let target_service_id = target.service_id.clone();
                let target_method_name = target.method_name.clone();
                let task = tokio::spawn(async move {
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
                        service_id: target_service_id,
                        method_name: target_method_name,
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

                tasks.push(task);
            }
        }

        // Wait for all parallel calls in this step to complete
        for task in tasks {
            task.await
                .map_err(|e| Status::internal(format!("Task join error: {}", e)))??;
        }
    }

    Ok(())
}

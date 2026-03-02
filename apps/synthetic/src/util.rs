use crate::config::{CallTarget, ServiceMethod};
use crate::service_registry::ServiceRegistry;
use app_utils::timing::time_now;
use rand::{thread_rng, Rng};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tonic::masa::{
    METHOD_NAME_OVERRIDE_HEADER, ORACLE_CHILD_LATENCY_US_HEADER,
    ORACLE_REMAINING_AFTER_CHILD_US_HEADER, ORACLE_SELF_WORK_US_HEADER,
    SERVICE_NAME_OVERRIDE_HEADER,
};
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

#[derive(Debug, Clone)]
pub struct OracleChildCall {
    pub target: CallTarget,
    pub child_total_us: u64,
    pub child_self_us: u64,
    pub remaining_after_child_us: u64,
}

#[derive(Debug, Clone, Copy)]
struct OracleMethodSample {
    self_us: u64,
    total_us: u64,
}

fn sample_oracle_method(
    target: &CallTarget,
    method_lookup: &HashMap<(String, String), ServiceMethod>,
    visiting: &mut HashSet<(String, String)>,
) -> Result<OracleMethodSample, Status> {
    let key = (target.service_id.clone(), target.method_name.clone());
    if !visiting.insert(key.clone()) {
        return Err(Status::internal(format!(
            "Cycle detected while building oracle plan at '{}::{}'",
            key.0, key.1
        )));
    }

    let method = method_lookup.get(&key).ok_or_else(|| {
        Status::not_found(format!(
            "Method '{}' not found in service '{}'",
            target.method_name, target.service_id
        ))
    })?;

    let self_us = method.latency_distribution.sample();
    let mut total_us = self_us;

    for step in &method.parsed_call_sequence {
        let mut step_critical_us = 0u64;
        for (child_target, probability) in step {
            if !should_make_call(*probability) {
                continue;
            }
            let child_sample = sample_oracle_method(child_target, method_lookup, visiting)?;
            step_critical_us = step_critical_us.max(child_sample.total_us);
        }
        total_us = total_us.saturating_add(step_critical_us);
    }

    visiting.remove(&key);
    Ok(OracleMethodSample { self_us, total_us })
}

pub fn build_oracle_call_plan(
    call_sequence: &[Vec<(CallTarget, f64)>],
    method_lookup: &HashMap<(String, String), ServiceMethod>,
    local_work_us: u64,
) -> Result<Vec<Vec<OracleChildCall>>, Status> {
    let mut sampled_steps: Vec<Vec<(CallTarget, OracleMethodSample)>> = Vec::new();
    let mut step_critical_paths = Vec::new();

    for step in call_sequence {
        let mut sampled_calls = Vec::new();
        let mut step_critical_path_us = 0u64;

        for (target, probability) in step {
            if !should_make_call(*probability) {
                continue;
            }

            let mut visiting = HashSet::new();
            let child_sample = sample_oracle_method(target, method_lookup, &mut visiting)?;
            step_critical_path_us = step_critical_path_us.max(child_sample.total_us);
            sampled_calls.push((target.clone(), child_sample));
        }

        sampled_steps.push(sampled_calls);
        step_critical_paths.push(step_critical_path_us);
    }

    let mut future_after_step_us = vec![0u64; step_critical_paths.len()];
    let mut suffix_sum = 0u64;
    for i in (0..step_critical_paths.len()).rev() {
        future_after_step_us[i] = suffix_sum;
        suffix_sum = suffix_sum.saturating_add(step_critical_paths[i]);
    }

    let mut plan = Vec::with_capacity(sampled_steps.len());
    for (idx, sampled_calls) in sampled_steps.into_iter().enumerate() {
        let step_critical = step_critical_paths[idx];
        let mut plan_step = Vec::with_capacity(sampled_calls.len());
        for (target, child_sample) in sampled_calls {
            let remaining_after_child_us = step_critical
                .saturating_sub(child_sample.total_us)
                .saturating_add(future_after_step_us[idx])
                .saturating_add(local_work_us);
            plan_step.push(OracleChildCall {
                target,
                child_total_us: child_sample.total_us,
                child_self_us: child_sample.self_us,
                remaining_after_child_us,
            });
        }
        plan.push(plan_step);
    }

    Ok(plan)
}

pub async fn execute_call_sequence(
    registry: &ServiceRegistry,
    call_sequence: &[Vec<(CallTarget, f64)>],
) -> Result<(), Status> {
    // Execute steps sequentially
    for step in call_sequence {
        // Structured fanout: each step owns its task set and joins it before proceeding.
        let mut tasks = JoinSet::new();

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
            }
        }

        // Wait for all calls in this step; dropping the set aborts unfinished tasks on error.
        while let Some(task_result) = tasks.join_next().await {
            let rpc_result =
                task_result.map_err(|e| Status::internal(format!("Task join error: {}", e)))?;
            rpc_result?;
        }
    }

    Ok(())
}

pub async fn execute_oracle_call_plan(
    registry: &ServiceRegistry,
    plan: &[Vec<OracleChildCall>],
) -> Result<(), Status> {
    for step in plan {
        let mut tasks = JoinSet::new();

        for call in step {
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
            let target_service_id = call.target.service_id.clone();
            let target_method_name = call.target.method_name.clone();
            let child_total_us = call.child_total_us;
            let child_self_us = call.child_self_us;
            let remaining_after_child_us = call.remaining_after_child_us;

            tasks.spawn(async move {
                let method_meta =
                    MetadataValue::try_from(target_method_name.as_str()).map_err(|e| {
                        Status::internal(format!("Failed to create method name override: {:?}", e))
                    })?;
                let service_meta =
                    MetadataValue::try_from(target_service_id.as_str()).map_err(|e| {
                        Status::internal(format!("Failed to create service name override: {:?}", e))
                    })?;
                let child_total = child_total_us.to_string();
                let child_total_meta =
                    MetadataValue::try_from(child_total.as_str()).map_err(|e| {
                        Status::internal(format!(
                            "Failed to create oracle child latency metadata: {:?}",
                            e
                        ))
                    })?;
                let child_self = child_self_us.to_string();
                let child_self_meta =
                    MetadataValue::try_from(child_self.as_str()).map_err(|e| {
                        Status::internal(format!(
                            "Failed to create oracle self work metadata: {:?}",
                            e
                        ))
                    })?;
                let remaining_latency = remaining_after_child_us.to_string();
                let rem_meta =
                    MetadataValue::try_from(remaining_latency.as_str()).map_err(|e| {
                        Status::internal(format!(
                            "Failed to create oracle remaining latency metadata: {:?}",
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
                request
                    .metadata_mut()
                    .insert(ORACLE_CHILD_LATENCY_US_HEADER, child_total_meta);
                request
                    .metadata_mut()
                    .insert(ORACLE_SELF_WORK_US_HEADER, child_self_meta);
                request
                    .metadata_mut()
                    .insert(ORACLE_REMAINING_AFTER_CHILD_US_HEADER, rem_meta);

                client.clone().handle_method(request).await
            });
        }

        while let Some(task_result) = tasks.join_next().await {
            let rpc_result =
                task_result.map_err(|e| Status::internal(format!("Task join error: {}", e)))?;
            rpc_result?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::LatencyDistribution;
    use rand_distr::WeightedIndex;

    fn target(service: &str, method: &str) -> CallTarget {
        CallTarget {
            service_id: service.to_string(),
            method_name: method.to_string(),
        }
    }

    fn fixed_latency(v: u64) -> LatencyDistribution {
        LatencyDistribution::Discrete {
            weights: vec![1.0],
            values: vec![v],
            dist: WeightedIndex::new(vec![1.0]).unwrap(),
        }
    }

    fn method(name: &str, self_us: u64, seq: Vec<Vec<(CallTarget, f64)>>) -> ServiceMethod {
        ServiceMethod {
            name: name.to_string(),
            latency_distribution: fixed_latency(self_us),
            call_sequence_raw: Vec::new(),
            parsed_call_sequence: seq,
            busy_spin_ratio: None,
        }
    }

    #[test]
    fn oracle_plan_leaf_child_sets_total_and_self() {
        let mut lookup = HashMap::new();
        lookup.insert(
            ("svc".to_string(), "leaf".to_string()),
            method("leaf", 100, vec![]),
        );
        let seq = vec![vec![(target("svc", "leaf"), 1.0)]];

        let plan = build_oracle_call_plan(&seq, &lookup, 20).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].len(), 1);
        assert_eq!(plan[0][0].child_self_us, 100);
        assert_eq!(plan[0][0].child_total_us, 100);
        assert_eq!(plan[0][0].remaining_after_child_us, 20);
    }

    #[test]
    fn oracle_plan_chain_child_total_includes_descendants() {
        let mut lookup = HashMap::new();
        lookup.insert(
            ("svc".to_string(), "leaf".to_string()),
            method("leaf", 200, vec![]),
        );
        lookup.insert(
            ("svc".to_string(), "mid".to_string()),
            method("mid", 50, vec![vec![(target("svc", "leaf"), 1.0)]]),
        );
        let seq = vec![vec![(target("svc", "mid"), 1.0)]];

        let plan = build_oracle_call_plan(&seq, &lookup, 30).unwrap();
        assert_eq!(plan[0][0].child_self_us, 50);
        assert_eq!(plan[0][0].child_total_us, 250);
        assert_eq!(plan[0][0].remaining_after_child_us, 30);
    }

    #[test]
    fn oracle_plan_parallel_and_suffix_remaining_math_uses_total() {
        let mut lookup = HashMap::new();
        lookup.insert(
            ("svc".to_string(), "a".to_string()),
            method("a", 100, vec![]),
        );
        lookup.insert(
            ("svc".to_string(), "b".to_string()),
            method("b", 300, vec![]),
        );
        lookup.insert(
            ("svc".to_string(), "c".to_string()),
            method("c", 50, vec![]),
        );
        let seq = vec![
            vec![(target("svc", "a"), 1.0), (target("svc", "b"), 1.0)],
            vec![(target("svc", "c"), 1.0)],
        ];

        let plan = build_oracle_call_plan(&seq, &lookup, 40).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].len(), 2);
        assert_eq!(plan[1].len(), 1);

        let call_a = plan[0]
            .iter()
            .find(|c| c.target.method_name == "a")
            .unwrap();
        let call_b = plan[0]
            .iter()
            .find(|c| c.target.method_name == "b")
            .unwrap();
        let call_c = &plan[1][0];

        assert_eq!(call_a.child_total_us, 100);
        assert_eq!(call_b.child_total_us, 300);
        assert_eq!(call_c.child_total_us, 50);
        assert_eq!(call_a.remaining_after_child_us, 290); // (300-100)+50+40
        assert_eq!(call_b.remaining_after_child_us, 90); // (300-300)+50+40
        assert_eq!(call_c.remaining_after_child_us, 40); // (50-50)+0+40
    }

    #[test]
    fn oracle_plan_detects_cycles() {
        let mut lookup = HashMap::new();
        lookup.insert(
            ("svc".to_string(), "x".to_string()),
            method("x", 10, vec![vec![(target("svc", "y"), 1.0)]]),
        );
        lookup.insert(
            ("svc".to_string(), "y".to_string()),
            method("y", 20, vec![vec![(target("svc", "x"), 1.0)]]),
        );
        let seq = vec![vec![(target("svc", "y"), 1.0)]];

        let err = build_oracle_call_plan(&seq, &lookup, 0).unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
        assert!(err.message().contains("Cycle detected"));
    }
}

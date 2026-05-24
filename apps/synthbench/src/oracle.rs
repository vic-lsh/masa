use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use masa::{ORACLE_CHILD_WORK_US_HEADER, ORACLE_REMAINING_AFTER_US_HEADER};
use rand::{thread_rng, Rng};
use serde::{Deserialize, Serialize};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::Status;

use crate::config::{
    CallGraphConfig, CallTarget, OnlineOracleOverheadConfig, OnlineOracleOverheadPlacement,
    OracleWorkEstimateConfig, ParsedCall, ServiceMethod,
};

const ORACLE_PLAN_HEADER: &str = "x-masa-oracle-plan";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OracleHint {
    pub child_work_us: u64,
    pub remaining_after_us: u64,
}

impl OracleHint {
    pub fn inject_headers(&self, metadata: &mut MetadataMap) -> Result<(), Status> {
        insert_u64_header(metadata, ORACLE_CHILD_WORK_US_HEADER, self.child_work_us)?;
        insert_u64_header(
            metadata,
            ORACLE_REMAINING_AFTER_US_HEADER,
            self.remaining_after_us,
        )?;
        Ok(())
    }
}

fn insert_u64_header(
    metadata: &mut MetadataMap,
    key: &'static str,
    value: u64,
) -> Result<(), Status> {
    let value = value.to_string();
    let metadata_value = MetadataValue::try_from(value.as_str()).map_err(|e| {
        Status::internal(format!("failed to encode oracle header '{}': {}", key, e))
    })?;
    metadata.insert(key, metadata_value);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedCall {
    pub target: CallTarget,
    pub hint: OracleHint,
    pub plan: ExecutionPlan,
}

impl PlannedCall {
    pub fn inject_headers(&self, metadata: &mut MetadataMap) -> Result<(), Status> {
        self.hint.inject_headers(metadata)?;
        self.plan.inject_header(metadata)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub local_work_us: u64,
    #[serde(default)]
    pub estimate_overhead_us: u64,
    pub child_steps: Vec<Vec<PlannedCall>>,
}

impl ExecutionPlan {
    pub fn actual_total_work_us(&self) -> u64 {
        let child_work_us = self
            .child_steps
            .iter()
            .map(|step| {
                step.iter()
                    .map(|call| call.plan.actual_total_work_us())
                    .max()
                    .unwrap_or(0)
            })
            .sum::<u64>();

        child_work_us.saturating_add(self.local_work_us)
    }

    pub fn estimated_total_work_us(&self) -> u64 {
        let child_work_us = self
            .child_steps
            .iter()
            .map(|step| {
                step.iter()
                    .map(|call| call.plan.estimated_total_work_us())
                    .max()
                    .unwrap_or(0)
            })
            .sum::<u64>();

        child_work_us
            .saturating_add(self.local_work_us)
            .saturating_add(self.estimate_overhead_us)
    }

    fn inject_header(&self, metadata: &mut MetadataMap) -> Result<(), Status> {
        let json = serde_json::to_string(self)
            .map_err(|e| Status::internal(format!("failed to encode oracle plan: {}", e)))?;
        let metadata_value = MetadataValue::try_from(json.as_str())
            .map_err(|e| Status::internal(format!("failed to store oracle plan: {}", e)))?;
        metadata.insert(ORACLE_PLAN_HEADER, metadata_value);
        Ok(())
    }

    pub fn from_metadata(metadata: &MetadataMap) -> Result<Option<Self>, Status> {
        let Some(value) = metadata.get(ORACLE_PLAN_HEADER) else {
            return Ok(None);
        };

        let json = value
            .to_str()
            .map_err(|e| Status::internal(format!("invalid oracle plan header: {}", e)))?;

        serde_json::from_str(json)
            .map(Some)
            .map_err(|e| Status::internal(format!("failed to decode oracle plan: {}", e)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MethodKey {
    service_id: String,
    method_name: String,
}

impl MethodKey {
    fn new(service_id: impl Into<String>, method_name: impl Into<String>) -> Self {
        Self {
            service_id: service_id.into(),
            method_name: method_name.into(),
        }
    }

    fn from_target(target: &CallTarget) -> Self {
        Self::new(target.service_id.clone(), target.method_name.clone())
    }

    fn display(&self) -> String {
        format!("{}::{}", self.service_id, self.method_name)
    }
}

#[derive(Debug, Clone)]
pub struct OraclePlanner {
    entry_points: HashMap<String, Vec<Vec<ParsedCall>>>,
    methods: HashMap<MethodKey, ServiceMethod>,
    estimate: OracleWorkEstimateConfig,
    calibrator: OnlineOverheadCalibrator,
}

impl OraclePlanner {
    pub fn new(
        config: &CallGraphConfig,
        estimate: OracleWorkEstimateConfig,
    ) -> Result<Self, String> {
        validate_supported_call_graph(config)?;

        let mut methods = HashMap::new();
        for service in &config.services {
            for method in &service.methods {
                let key = MethodKey::new(service.id.clone(), method.name.clone());
                methods.insert(key, method.clone());
            }
        }

        Ok(Self {
            entry_points: config.parsed_entry_points.clone(),
            methods,
            calibrator: OnlineOverheadCalibrator::new(estimate.online_overhead.clone()),
            estimate,
        })
    }

    pub fn plan_entry_point(&self, entry_point: &str) -> Result<ExecutionPlan, String> {
        let sequence = self.entry_points.get(entry_point).ok_or_else(|| {
            format!(
                "missing entry point '{}' while building oracle execution plan",
                entry_point
            )
        })?;
        let mut visiting = HashSet::new();
        let mut plan = self.plan_sequence(sequence, 0, &mut visiting)?;
        self.calibrator.apply(entry_point, &mut plan);
        Ok(plan)
    }

    pub fn record_entry_point_latency(
        &self,
        entry_point: &str,
        observed_latency_us: u64,
        plan: &ExecutionPlan,
    ) {
        let observed_overhead_us = observed_latency_us.saturating_sub(plan.actual_total_work_us());
        self.calibrator.record(entry_point, observed_overhead_us);
    }

    fn plan_method(
        &self,
        key: &MethodKey,
        visiting: &mut HashSet<MethodKey>,
    ) -> Result<ExecutionPlan, String> {
        if !visiting.insert(key.clone()) {
            return Err(format!(
                "sched_oracle does not support recursive call graphs; cycle includes {}",
                key.display()
            ));
        }

        let method = self.methods.get(key).ok_or_else(|| {
            format!(
                "method {} not found while building oracle execution plan",
                key.display()
            )
        })?;

        let local_work_us = method.latency_distribution.sample();
        let estimate_overhead_us = self
            .estimate
            .overhead_for(&key.service_id, &key.method_name);
        let plan = self.plan_sequence(&method.parsed_call_sequence, local_work_us, visiting);
        visiting.remove(key);
        plan.map(|mut plan| {
            plan.estimate_overhead_us = estimate_overhead_us;
            plan
        })
    }

    fn plan_sequence(
        &self,
        sequence: &[Vec<ParsedCall>],
        local_work_us: u64,
        visiting: &mut HashSet<MethodKey>,
    ) -> Result<ExecutionPlan, String> {
        let mut child_steps = Vec::with_capacity(sequence.len());

        for step in sequence {
            let mut planned_step = Vec::new();

            if let Some(call) = step.first() {
                if should_make_call(call.probability) {
                    let key = MethodKey::from_target(&call.target);
                    let plan = self.plan_method(&key, visiting)?;
                    planned_step.push(PlannedCall {
                        target: call.target.clone(),
                        hint: OracleHint {
                            child_work_us: 0,
                            remaining_after_us: 0,
                        },
                        plan,
                    });
                }
            }

            child_steps.push(planned_step);
        }

        attach_hints(local_work_us, &mut child_steps);

        Ok(ExecutionPlan {
            local_work_us,
            estimate_overhead_us: 0,
            child_steps,
        })
    }
}

fn should_make_call(probability: f64) -> bool {
    thread_rng().gen::<f64>() < probability
}

fn attach_hints(local_work_us: u64, child_steps: &mut [Vec<PlannedCall>]) {
    let mut remaining_after_us = local_work_us;

    for step in child_steps.iter_mut().rev() {
        let step_work_us = step
            .iter()
            .map(|call| call.plan.estimated_total_work_us())
            .max()
            .unwrap_or(0);

        for call in step {
            call.hint = OracleHint {
                child_work_us: call.plan.estimated_total_work_us(),
                remaining_after_us,
            };
        }

        remaining_after_us = remaining_after_us.saturating_add(step_work_us);
    }
}

#[derive(Debug, Clone)]
struct OnlineOverheadCalibrator {
    config: OnlineOracleOverheadConfig,
    overheads: Arc<Mutex<HashMap<String, f64>>>,
}

impl OnlineOverheadCalibrator {
    fn new(config: OnlineOracleOverheadConfig) -> Self {
        Self {
            config,
            overheads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn apply(&self, entry_point: &str, plan: &mut ExecutionPlan) {
        if !self.config.enabled {
            return;
        }

        let overhead_us = self.current_overhead_us(entry_point);
        if overhead_us == 0 {
            return;
        }

        match self.config.placement {
            OnlineOracleOverheadPlacement::LastCall => {
                add_overhead_to_last_call(plan, overhead_us);
            }
            OnlineOracleOverheadPlacement::UniformCalls => {
                add_overhead_uniformly(plan, overhead_us);
            }
        }
        refresh_hints(plan);
    }

    fn record(&self, entry_point: &str, observed_overhead_us: u64) {
        if !self.config.enabled {
            return;
        }

        let sample = if self.config.max_overhead_us > 0 {
            observed_overhead_us.min(self.config.max_overhead_us)
        } else {
            observed_overhead_us
        };
        let alpha = self.config.alpha.clamp(0.0, 1.0);
        if alpha == 0.0 {
            return;
        }

        let mut overheads = self
            .overheads
            .lock()
            .expect("oracle overhead mutex poisoned");
        overheads
            .entry(entry_point.to_string())
            .and_modify(|current| {
                *current += alpha * (sample as f64 - *current);
            })
            .or_insert(sample as f64);
    }

    fn current_overhead_us(&self, entry_point: &str) -> u64 {
        self.overheads
            .lock()
            .expect("oracle overhead mutex poisoned")
            .get(entry_point)
            .copied()
            .unwrap_or(0.0)
            .round() as u64
    }
}

fn refresh_hints(plan: &mut ExecutionPlan) {
    for step in &mut plan.child_steps {
        for call in step {
            refresh_hints(&mut call.plan);
        }
    }
    attach_hints(plan.local_work_us, &mut plan.child_steps);
}

fn add_overhead_to_last_call(plan: &mut ExecutionPlan, overhead_us: u64) -> bool {
    for step in plan.child_steps.iter_mut().rev() {
        for call in step.iter_mut().rev() {
            if add_overhead_to_last_call(&mut call.plan, overhead_us) {
                return true;
            }
            call.plan.estimate_overhead_us =
                call.plan.estimate_overhead_us.saturating_add(overhead_us);
            return true;
        }
    }
    false
}

fn add_overhead_uniformly(plan: &mut ExecutionPlan, overhead_us: u64) {
    let call_count = count_planned_calls(plan);
    if call_count == 0 {
        return;
    }

    let per_call = overhead_us / call_count as u64;
    let mut remainder = overhead_us % call_count as u64;
    add_overhead_uniformly_inner(plan, per_call, &mut remainder);
}

fn count_planned_calls(plan: &ExecutionPlan) -> usize {
    plan.child_steps
        .iter()
        .flat_map(|step| step.iter())
        .map(|call| 1 + count_planned_calls(&call.plan))
        .sum()
}

fn add_overhead_uniformly_inner(plan: &mut ExecutionPlan, per_call: u64, remainder: &mut u64) {
    for step in &mut plan.child_steps {
        for call in step {
            let extra = if *remainder > 0 {
                *remainder -= 1;
                1
            } else {
                0
            };
            call.plan.estimate_overhead_us = call
                .plan
                .estimate_overhead_us
                .saturating_add(per_call.saturating_add(extra));
            add_overhead_uniformly_inner(&mut call.plan, per_call, remainder);
        }
    }
}

pub fn validate_supported_call_graph(config: &CallGraphConfig) -> Result<(), String> {
    for (entry_point, sequence) in &config.parsed_entry_points {
        validate_sequence_shape(sequence, &format!("entry point '{}'", entry_point))?;
    }

    let mut methods = HashMap::new();
    for service in &config.services {
        for method in &service.methods {
            validate_sequence_shape(
                &method.parsed_call_sequence,
                &format!("{}::{}", service.id, method.name),
            )?;

            let key = MethodKey::new(service.id.clone(), method.name.clone());
            methods.insert(key, method);
        }
    }

    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();
    for key in methods.keys() {
        validate_acyclic(key, &methods, &mut visited, &mut visiting)?;
    }

    Ok(())
}

fn validate_sequence_shape(sequence: &[Vec<ParsedCall>], owner: &str) -> Result<(), String> {
    for step in sequence {
        if step.len() > 1 {
            return Err(format!(
                "sched_oracle currently requires linear call graph steps; '{}' has fanout width {}",
                owner,
                step.len()
            ));
        }

        for call in step {
            if !(0.0..=1.0).contains(&call.probability) {
                return Err(format!(
                    "sched_oracle requires call probabilities in [0, 1]; '{} -> {}::{}' has probability {}",
                    owner, call.target.service_id, call.target.method_name, call.probability
                ));
            }
        }
    }

    Ok(())
}

fn validate_acyclic<'a>(
    key: &MethodKey,
    methods: &HashMap<MethodKey, &'a ServiceMethod>,
    visited: &mut HashSet<MethodKey>,
    visiting: &mut HashSet<MethodKey>,
) -> Result<(), String> {
    if visited.contains(key) {
        return Ok(());
    }

    if !visiting.insert(key.clone()) {
        return Err(format!(
            "sched_oracle does not support recursive call graphs; cycle includes {}",
            key.display()
        ));
    }

    let method = methods.get(key).ok_or_else(|| {
        format!(
            "method {} not found during oracle validation",
            key.display()
        )
    })?;

    for step in &method.parsed_call_sequence {
        if let Some(call) = step.first() {
            let child_key = MethodKey::from_target(&call.target);
            validate_acyclic(&child_key, methods, visited, visiting)?;
        }
    }

    visiting.remove(key);
    visited.insert(key.clone());
    Ok(())
}

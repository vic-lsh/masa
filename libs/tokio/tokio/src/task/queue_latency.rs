use crate::runtime::task::current_task_queue_latency;
use std::time::Duration;

/// Obtain the current task's queue latency.
pub fn obtain_task_queue_latency() -> Duration {
    current_task_queue_latency()
}
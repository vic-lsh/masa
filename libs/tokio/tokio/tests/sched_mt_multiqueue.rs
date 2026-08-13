#![cfg(all(
    feature = "sched_mt_multiqueue",
    feature = "rt-multi-thread",
    feature = "sync"
))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::runtime;
use tokio::sync::Barrier;
use tokio::task::TaskPriority;

#[test]
fn concurrent_wakeups_and_reprioritization_complete_exactly_once() {
    const WORKERS: usize = 16;
    const TASKS: usize = 4_096;
    const YIELDS_PER_TASK: usize = 32;

    let runtime = runtime::Builder::new_multi_thread()
        .worker_threads(WORKERS)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let start = Arc::new(Barrier::new(TASKS + 1));
        let completed = Arc::new((0..TASKS).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let mut handles = Vec::with_capacity(TASKS);

        for task_id in 0..TASKS {
            let start = Arc::clone(&start);
            let completed = Arc::clone(&completed);
            handles.push(tokio::task::spawn_with_prio(
                async move {
                    start.wait().await;
                    for iteration in 0..YIELDS_PER_TASK {
                        if iteration == YIELDS_PER_TASK / 2 {
                            tokio::task::reprioritize(TaskPriority::new((TASKS - task_id) as u64));
                        }
                        tokio::task::yield_now().await;
                    }
                    assert_eq!(
                        completed[task_id].fetch_add(1, Ordering::Relaxed),
                        0,
                        "task {task_id} completed more than once"
                    );
                },
                TaskPriority::new((task_id + 1) as u64),
            ));
        }

        start.wait().await;
        for handle in handles {
            handle.await.unwrap();
        }

        assert!(
            completed
                .iter()
                .all(|count| count.load(Ordering::Relaxed) == 1),
            "at least one task was lost"
        );
    });
}

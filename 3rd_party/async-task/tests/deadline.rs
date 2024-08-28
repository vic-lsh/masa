use async_task::{Runnable, Task};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tonic_masa::DeadlineHint;

fn spawn_util() -> (Runnable<()>, Task<()>) {
    fn dispatch(trampoline: extern "C" fn(NonNull<()>), context: NonNull<()>) {
        trampoline(context)
    }
    extern "C" fn trampoline(runnable: NonNull<()>) {
        let task = unsafe { Runnable::<()>::from_raw(runnable) };
        task.run();
    }

    let task_got_executed = Arc::new(AtomicBool::new(false));
    async_task::spawn(
        {
            let task_got_executed = task_got_executed.clone();
            async move { task_got_executed.store(true, Ordering::SeqCst) }
        },
        |runnable: Runnable<()>| dispatch(trampoline, runnable.into_raw()),
    )
}

fn sched_noop(_r: Runnable<()>) {}

#[test]
fn test_default_ddl() {
    let (runnable, task) = spawn_util();
    assert_eq!(runnable.deadline(), DeadlineHint::infra());
    assert_eq!(task.deadline(), DeadlineHint::infra());
}

#[test]
fn test_custom_ddl() {
    let ddl = DeadlineHint::new(100);
    let (runnable, task) = async_task::spawn_with_deadline(async {}, ddl, sched_noop);

    assert_eq!(runnable.deadline(), ddl);
    assert_eq!(task.deadline(), ddl);
}

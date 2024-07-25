use async_task::{Builder, Runnable};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tonic_deadline::DeadlineHint;

fn runnable_from_raw() -> Runnable<()> {
    fn dispatch(trampoline: extern "C" fn(NonNull<()>), context: NonNull<()>) {
        trampoline(context)
    }
    extern "C" fn trampoline(runnable: NonNull<()>) {
        let task = unsafe { Runnable::<()>::from_raw(runnable) };
        task.run();
    }

    let task_got_executed = Arc::new(AtomicBool::new(false));
    let (runnable, _handle) = async_task::spawn(
        {
            let task_got_executed = task_got_executed.clone();
            async move { task_got_executed.store(true, Ordering::SeqCst) }
        },
        |runnable: Runnable<()>| dispatch(trampoline, runnable.into_raw()),
    );
    runnable
}

#[test]
fn runnable_default_ddl() {
    let r = runnable_from_raw();
    assert_eq!(r.ddl(), DeadlineHint::infra());
}

#[test]
fn runnable_custom_ddl() {
    async fn my_future() {}
    let ddl = DeadlineHint::new(100);
    let (r, _) = unsafe {
        Builder::new()
            .deadline(ddl)
            .spawn_unchecked(move |()| my_future(), |_r: Runnable<()>| {})
    };

    assert_eq!(r.ddl(), ddl);
}

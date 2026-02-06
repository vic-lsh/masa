#![cfg(all(feature = "rt", feature = "sync", feature = "time", feature = "prio_global"))]
#![allow(unknown_lints, unexpected_cfgs)]

use masa_core::PriorityHint;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::runtime::Builder;
use tokio::sync::Notify;

thread_local! {
    static HOOK_ACTIVE: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

#[test]
fn spawn_with_prio_orders_tasks_by_hint() {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    rt.block_on(async {
        let notify = Arc::new(Notify::new());
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut handles = Vec::new();

        for (label, prio) in [("first", 10_u64), ("second", 50_u64), ("third", 90_u64)] {
            let notify = notify.clone();
            let order = order.clone();
            handles.push(tokio::task::spawn_with_prio(
                async move {
                    notify.notified().await;
                    order.lock().unwrap().push(label);
                },
                PriorityHint::new(prio),
            ));
        }

        tokio::task::yield_now().await;
        notify.notify_waiters();

        for handle in handles {
            handle.await.unwrap();
        }

        let recorded = order.lock().unwrap().clone();
        assert_eq!(recorded, vec!["first", "second", "third"]);
    });
}

#[test]
fn infrastructure_priority_runs_first() {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();

    rt.block_on(async {
        let notify = Arc::new(Notify::new());
        let order = Arc::new(Mutex::new(Vec::new()));

        let infra_notify = notify.clone();
        let infra_order = order.clone();
        let infra = tokio::spawn(async move {
            infra_notify.notified().await;
            infra_order.lock().unwrap().push("infra");
        });

        let user_notify = notify.clone();
        let user_order = order.clone();
        let user = tokio::task::spawn_with_prio(
            async move {
                user_notify.notified().await;
                user_order.lock().unwrap().push("user");
            },
            PriorityHint::new(50),
        );

        tokio::task::yield_now().await;
        notify.notify_waiters();

        infra.await.unwrap();
        user.await.unwrap();

        let recorded = order.lock().unwrap().clone();
        assert_eq!(recorded, vec!["infra", "user"]);
    });
}

#[test]
fn poll_hook_propagates_to_child_tasks() {
    let rt = Builder::new_current_thread().enable_all().build().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));

    rt.block_on(async {
        let hits = hits.clone();
        tokio::spawn(async move {
            unsafe {
                let hook = tokio::task::PollHook::new(
                    core::ptr::null(),
                    None,
                    None,
                    Some(before_hook),
                    Some(after_hook),
                );
                assert!(tokio::configure_child_task_poll_hook(hook));
            }

            let mut handles = Vec::new();
            for _ in 0..3 {
                let hits = hits.clone();
                handles.push(tokio::task::spawn(async move {
                    assert!(HOOK_ACTIVE.with(|cell| cell.get()));
                    hits.fetch_add(1, Ordering::SeqCst);
                }));
            }

            for handle in handles {
                handle.await.unwrap();
            }

            assert!(tokio::reset_child_task_poll_hook());
        })
        .await
        .unwrap();
    });

    assert_eq!(hits.load(Ordering::SeqCst), 3);
    HOOK_ACTIVE.with(|cell| assert!(!cell.get()));
}

fn before_hook(_ctx: *const ()) {
    HOOK_ACTIVE.with(|cell| cell.set(true));
}

fn after_hook(_ctx: *const ()) {
    HOOK_ACTIVE.with(|cell| cell.set(false));
}

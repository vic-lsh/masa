use tokio::runtime::Builder;
use tokio::SchedFlavor;

#[test]
fn test_default_policy_is_fifo() {
    let rt = Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        assert_eq!(tokio::get_sched_flavor(), SchedFlavor::Fifo);
    });
}

#[test]
fn test_prio_policy() {
    type PrioPolicy = masa::CompositePolicy<masa::Prio, masa::EarlyReturnDisabled, masa::DeadlinePolicyNone>;
    
    let rt = Builder::new_current_thread()
        .policy::<PrioPolicy>()
        .build()
        .unwrap();
    
    rt.block_on(async {
        assert_eq!(tokio::get_sched_flavor(), SchedFlavor::Prio);
    });
}

#[test]
fn test_prio_oldest_policy() {
    type PrioOldestPolicy = masa::CompositePolicy<masa::PrioOldest, masa::EarlyReturnDisabled, masa::DeadlinePolicyNone>;
    
    let rt = Builder::new_current_thread()
        .policy::<PrioOldestPolicy>()
        .build()
        .unwrap();
    
    rt.block_on(async {
        // PrioOldest maps to Prio flavor in the current implementation of get_sched_flavor
        // (as seen in libs/tokio/tokio/src/runtime/scheduler/current_thread/queue/mod.rs)
        assert_eq!(tokio::get_sched_flavor(), SchedFlavor::Prio);
    });
}
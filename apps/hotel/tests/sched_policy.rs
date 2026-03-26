#[cfg(not(any(feature = "sched_fifo", feature = "sched_prio",)))]
#[test]
fn test_default_policy_is_fifo() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Fifo);
}

#[cfg(feature = "sched_fifo")]
#[test]
fn test_fifo_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Fifo);
}

#[cfg(feature = "sched_prio")]
#[test]
fn test_sched_prio_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

#[cfg(all(feature = "sched_prio", feature = "tailclipper"))]
#[test]
fn test_tailclipper_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

#[cfg(all(feature = "sched_prio", feature = "est_abort"))]
#[test]
fn test_est_abort_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

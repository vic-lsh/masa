#[cfg(not(any(feature = "fifo", feature = "prio_global", feature = "prio_local")))]
#[test]
fn test_default_policy_is_fifo() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Fifo);
}

#[cfg(feature = "fifo")]
#[test]
fn test_fifo_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Fifo);
}

#[cfg(feature = "prio_global")]
#[test]
fn test_prio_global_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

#[cfg(feature = "prio_local")]
#[test]
fn test_prio_local_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

#[cfg(not(any(
    feature = "sched_fifo",
    feature = "sched_slo",
    feature = "sched_tailclipper"
)))]
#[test]
fn test_default_policy_is_fifo() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Fifo);
}

#[cfg(feature = "sched_fifo")]
#[test]
fn test_fifo_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Fifo);
}

#[cfg(feature = "sched_slo")]
#[test]
fn test_sched_slo_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

#[cfg(feature = "sched_tailclipper")]
#[test]
fn test_sched_tailclipper_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

#[cfg(feature = "sched_pred")]
#[test]
fn test_sched_pred_policy() {
    assert_eq!(tokio::get_sched_flavor(), tokio::SchedFlavor::Prio);
}

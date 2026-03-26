pub const SCHED_PRIO: bool = cfg!(feature = "sched_prio");

pub const SCHED_FIFO: bool = cfg!(feature = "sched_fifo");

pub const TAILCLIPPER: bool = cfg!(feature = "tailclipper");

pub const SLO_ABORT: bool = cfg!(feature = "slo_abort");

pub const EST_ABORT: bool = cfg!(feature = "est_abort");

#[allow(dead_code)]
pub const RAJOMON: bool = cfg!(feature = "ac_rajomon");

// === Scheduling discipline constraints ===
// sched_fifo and sched_prio select different runtime queue implementations
// (FIFO vs BinaryHeap). Enabling both would create conflicting type aliases.
#[cfg(all(feature = "sched_fifo", feature = "sched_prio"))]
compile_error!("Enable at most one scheduling discipline: sched_fifo | sched_prio");

// === TailClipper constraints ===
// tailclipper implements the TailClipper paper's scheduling policy exactly:
// sched_prio + round-robin fairness for the top-N priority tasks.
// Adding est_abort or ac_est would modify the paper's original design,
// so tailclipper is only valid with sched_prio (+ optionally slo_abort).
#[cfg(all(feature = "tailclipper", feature = "est_abort"))]
compile_error!(
    "'tailclipper' cannot be combined with 'est_abort': \
    tailclipper implements the TailClipper paper's policy as-is"
);

#[cfg(all(feature = "tailclipper", feature = "ac_est"))]
compile_error!(
    "'tailclipper' cannot be combined with 'ac_est': \
    tailclipper implements the TailClipper paper's policy as-is"
);

// === Admission control constraints ===
// ac_est (estimation-based) and ac_rajomon (token-based) are two different admission
// control strategies. Only one can be active at a time.
#[cfg(all(feature = "ac_est", feature = "ac_rajomon"))]
compile_error!("Enable at most one admission control strategy: ac_est | ac_rajomon");

// ac_est uses compute-time feasibility and efficiency-based admission checks.
// These checks need SLO_ABORT to be meaningful (they decide whether to abort
// requests that are predicted to miss their SLO).
#[cfg(all(feature = "ac_est", not(feature = "slo_abort")))]
compile_error!("Feature 'ac_est' requires 'slo_abort'");

// early + ac_rajomon can now be combined: ac_rajomon handles admission control
// (token-based), slo_abort/est_abort handle in-flight abortion (deadline-based).

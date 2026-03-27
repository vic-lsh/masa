pub const SCHED_FIFO: bool = cfg!(feature = "sched_fifo");

pub const SCHED_SLO: bool = cfg!(feature = "sched_slo");

pub const SCHED_TAILCLIPPER: bool = cfg!(feature = "sched_tailclipper");

pub const SCHED_PRED: bool = cfg!(feature = "sched_pred");

pub const SLO_ABORT: bool = cfg!(feature = "slo_abort");

#[allow(dead_code)]
pub const RAJOMON: bool = cfg!(feature = "ac_rajomon");

// === Scheduling discipline constraints ===
// The base scheduling policies are mutually exclusive.
#[cfg(all(feature = "sched_fifo", feature = "sched_slo"))]
compile_error!("Enable at most one scheduling policy: sched_fifo | sched_slo | sched_tailclipper");

#[cfg(all(feature = "sched_fifo", feature = "sched_tailclipper"))]
compile_error!("Enable at most one scheduling policy: sched_fifo | sched_slo | sched_tailclipper");

#[cfg(all(feature = "sched_slo", feature = "sched_tailclipper"))]
compile_error!("Enable at most one scheduling policy: sched_slo | sched_tailclipper");

// === TailClipper constraints ===
// sched_tailclipper implements the TailClipper paper's scheduling policy exactly:
// sched_prio + round-robin fairness for the top-N priority tasks.
// Adding sched_pred or ac_est would modify the paper's original design.
#[cfg(all(feature = "sched_tailclipper", feature = "sched_pred"))]
compile_error!(
    "'sched_tailclipper' cannot be combined with 'sched_pred': \
    sched_tailclipper implements the TailClipper paper's policy as-is"
);

#[cfg(all(feature = "sched_tailclipper", feature = "ac_est"))]
compile_error!(
    "'sched_tailclipper' cannot be combined with 'ac_est': \
    sched_tailclipper implements the TailClipper paper's policy as-is"
);

// === Admission control constraints ===
// ac_est (estimation-based) and ac_rajomon (token-based) are two different admission
// control strategies. Only one can be active at a time.
#[cfg(all(feature = "ac_est", feature = "ac_rajomon"))]
compile_error!("Enable at most one admission control strategy: ac_est | ac_rajomon");

// Admission control requires a scheduling policy to be active, otherwise
// DefaultHooks resolves to NoopHooks and the overlay is never invoked.
#[cfg(all(
    any(feature = "ac_est", feature = "ac_rajomon"),
    not(any(
        feature = "sched_fifo",
        feature = "sched_slo",
        feature = "sched_tailclipper"
    ))
))]
compile_error!(
    "Admission control (ac_est | ac_rajomon) requires a scheduling policy \
     (sched_fifo | sched_slo | sched_tailclipper)"
);

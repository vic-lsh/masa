pub const SCHED_FIFO: bool = cfg!(feature = "sched_fifo");

pub const SCHED_SLO: bool = cfg!(feature = "sched_slo");

pub const SCHED_TAILCLIPPER: bool = cfg!(feature = "sched_tailclipper");

pub const SCHED_PRED: bool = cfg!(feature = "sched_pred");

pub const ABORT_SLO: bool = cfg!(feature = "abort_slo");

pub const ABORT_SLACK: bool = cfg!(feature = "abort_slack");

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
// Adding sched_pred or ac_pred would modify the paper's original design.
#[cfg(all(feature = "sched_tailclipper", feature = "sched_pred"))]
compile_error!(
    "'sched_tailclipper' cannot be combined with 'sched_pred': \
    sched_tailclipper implements the TailClipper paper's policy as-is"
);

#[cfg(all(feature = "sched_tailclipper", feature = "ac_pred"))]
compile_error!(
    "'sched_tailclipper' cannot be combined with 'ac_pred': \
    sched_tailclipper implements the TailClipper paper's policy as-is"
);

// === Admission control constraints ===
// ac_pred (estimation-based) and ac_rajomon (token-based) are two different admission
// control strategies. Only one can be active at a time.
#[cfg(all(feature = "ac_pred", feature = "ac_rajomon"))]
compile_error!("Enable at most one admission control strategy: ac_pred | ac_rajomon");

// === Abort constraints ===
// abort_slack is a superset of abort_slo; enabling both explicitly is redundant
// but not an error since abort_slack implies abort_slo via Cargo deps.
// abort_slack requires estimator for predictive abort checks.
#[cfg(all(feature = "abort_slack", not(feature = "estimator")))]
compile_error!("'abort_slack' requires 'estimator' for predictive abort checks");

#[cfg(all(feature = "abort_slack", feature = "abort_slo"))]
compile_error!("Enabling both 'abort_slack' and 'abort_slo' is redundant since 'abort_slack' implies 'abort_slo'");

// === Estimator constraints ===
// sched_pred and ac_pred require the estimator infrastructure.
#[cfg(all(feature = "sched_pred", not(feature = "estimator")))]
compile_error!("'sched_pred' requires 'estimator'");

#[cfg(all(feature = "ac_pred", not(feature = "estimator")))]
compile_error!("'ac_pred' requires 'estimator'");

// Admission control requires a scheduling policy to be active, otherwise
// DefaultHooks resolves to NoopHooks and the layer is never invoked.
#[cfg(all(
    any(feature = "ac_pred", feature = "ac_rajomon"),
    not(any(
        feature = "sched_fifo",
        feature = "sched_slo",
        feature = "sched_tailclipper"
    ))
))]
compile_error!(
    "Admission control (ac_pred | ac_rajomon) requires a scheduling policy \
     (sched_fifo | sched_slo | sched_tailclipper)"
);

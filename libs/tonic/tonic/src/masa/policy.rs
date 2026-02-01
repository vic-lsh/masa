use masa::{
    CompositePolicy, DeadlinePolicyGlobal, DeadlinePolicyLocal, DeadlinePolicyNone,
    DeadlinePolicyOldest,
};
use crate::masa::context::{
    LocalDeadlinePolicy, NoopMasaHooks, PrioOldest, QueueGlobal, MasaHooks,
};

/// A policy that can be used with Tonic.
pub trait TonicPolicy: masa::Policy {
    /// The hooks implementation associated with this policy.
    type Hooks: MasaHooks;
}

// Map DeadlinePolicyNone -> NoopMasaHooks
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyNone>
where
    Q: masa::Queue,
{
    type Hooks = NoopMasaHooks;
}

// Map DeadlinePolicyLocal -> LocalDeadlinePolicy
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyLocal>
where
    Q: masa::Queue,
{
    type Hooks = LocalDeadlinePolicy<Self>;
}

// Map DeadlinePolicyGlobal -> QueueGlobal
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyGlobal>
where
    Q: masa::Queue,
{
    type Hooks = QueueGlobal<Self>;
}

// Map DeadlinePolicyOldest -> PrioOldest
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyOldest>
where
    Q: masa::Queue,
{
    type Hooks = PrioOldest<Self>;
}

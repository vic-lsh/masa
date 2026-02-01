use masa::{
    CompositePolicy, DeadlinePolicyGlobal, DeadlinePolicyLocal, DeadlinePolicyNone,
    DeadlinePolicyOldest, EarlyReturnMode,
};
use crate::masa::context::{
    LocalDeadlinePolicy, MapEarlyReturn, NoopMasaHooks, PrioOldest, QueueGlobal, MasaHooks,
};

/// A policy that can be used with Tonic.
pub trait TonicPolicy: masa::Policy {
    /// The hooks implementation associated with this policy.
    type Hooks: MasaHooks;
}

// Map DeadlinePolicyNone -> NoopMasaHooks
impl<Q, E> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyNone>
where
    Q: masa::Queue,
    E: EarlyReturnMode,
{
    type Hooks = NoopMasaHooks;
}

// Map DeadlinePolicyLocal -> LocalDeadlinePolicy
impl<Q, E> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyLocal>
where
    Q: masa::Queue,
    E: EarlyReturnMode + MapEarlyReturn,
{
    type Hooks = LocalDeadlinePolicy<Self>;
}

// Map DeadlinePolicyGlobal -> QueueGlobal
impl<Q, E> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyGlobal>
where
    Q: masa::Queue,
    E: EarlyReturnMode + MapEarlyReturn,
{
    type Hooks = QueueGlobal<Self>;
}

// Map DeadlinePolicyOldest -> PrioOldest
impl<Q, E> TonicPolicy for CompositePolicy<Q, E, DeadlinePolicyOldest>
where
    Q: masa::Queue,
    E: EarlyReturnMode + MapEarlyReturn,
{
    type Hooks = PrioOldest<Self>;
}

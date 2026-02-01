# Compositional Policy System Refactoring Plan

## Goal
Implement a policy selection system that allows:
1.  **Compositional Configuration**: Select Queueing Discipline, Early Return, and Deadline Strategy independently via CLI flags.
2.  **Zero Runtime Overhead**: Use compile-time monomorphization (Generics) to eliminate dynamic dispatch in the hot path.
3.  **No Public API Breakage**: Preserve `tokio` and `tonic` public APIs by hiding implementation details behind the `Runtime` and `Server` builders.

## Architecture Overview

The system uses a **Composite Policy Pattern**. A generic `CompositePolicy<Q, E, S>` struct acts as the carrier for all configuration types. Macros are used to bridge the gap between runtime CLI arguments and compile-time generic types.

### 1. `libs/masa`: Policy Definitions
Defines the dimensions of the policy and the composite carrier.

*   **Enums (Runtime)**: `QueueType`, `DeadlineStrategyType`.
*   **Traits (Compile-time)**: `Policy`, `DeadlineStrategyMarker`.
*   **Structs**: `CompositePolicy<Q, const E: bool, S>`.

### 2. `libs/tonic`: Logic Mapping
Maps the abstract strategies from `masa` to concrete Hook implementations in `tonic`.

*   **Trait**: `TonicPolicy` (extends `masa::Policy`).
*   **Implementations**:
    *   `StrategyNone` -> `NoopHooks`
    *   `StrategyLocal` -> `LocalDeadlinePolicy`
    *   `StrategyGlobal` -> `QueueGlobal`
    *   `StrategyOldest` -> `PrioOldest`

### 3. `apps/app-utils`: Glue Code
Provides reusable components for applications.

*   **Struct**: `PolicyArgs` (StructOpt compliant).
*   **Macro**: `launch_masa_server!` (Handles the dispatch from `enum` to `generic`).

### 4. `libs/tokio`: Queue Exposure
*   **Action**: Ensure `FifoQueue`, `BinaryHeapQueue`, `BinaryHeapRoundRobinQueue` are public (or reachable).

---

## Detailed Implementation Steps

### Step 1: Core Definitions in `libs/masa`

In `libs/masa/src/lib.rs`:

```rust
pub trait Queue { /* Marker or re-export of tokio trait */ }

/// Runtime configuration enum for Queue Discipline
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueType {
    Fifo,
    Prio,       // BinaryHeap
    PrioOldest, // BinaryHeapRoundRobin
}

/// Runtime configuration enum for Deadline Strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineStrategyType {
    None,
    Local,
    Global,
    Oldest,
}

pub trait Policy: 'static + Send + Sync + Copy {
    type Queue: Queue;
    const EARLY_RETURN: bool;
}

/// A composite policy struct.
/// Q: Queue Type, E: Early Return (bool), S: Deadline Strategy Marker
pub struct CompositePolicy<Q, const E: bool, S>(std::marker::PhantomData<(Q, S)>);

impl<Q, const E: bool, S> Policy for CompositePolicy<Q, E, S>
where
    Q: Queue,
    S: DeadlineStrategyMarker,
{
    type Queue = Q;
    const EARLY_RETURN: bool = E;
}

// Marker traits for strategies
pub trait DeadlineStrategyMarker: 'static + Send + Sync + Copy {}
pub struct StrategyNone; impl DeadlineStrategyMarker for StrategyNone {}
pub struct StrategyLocal; impl DeadlineStrategyMarker for StrategyLocal {}
pub struct StrategyGlobal; impl DeadlineStrategyMarker for StrategyGlobal {}
pub struct StrategyOldest; impl DeadlineStrategyMarker for StrategyOldest {}
```

### Step 2: Tonic Integration in `libs/tonic`

In `libs/tonic/tonic/src/masa/policy.rs`:

```rust
use masa::{CompositePolicy, StrategyNone, StrategyLocal, StrategyGlobal, StrategyOldest};
use crate::masa::hooks::{NoopHooks, LocalDeadlinePolicy, QueueGlobal, PrioOldest};

pub trait TonicPolicy: masa::Policy {
    type Hooks: crate::masa::MasaHooks;
}

// Map StrategyNone -> NoopHooks
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, StrategyNone> 
where Q: masa::Queue 
{
    type Hooks = NoopHooks;
}

// Map StrategyLocal -> LocalDeadlinePolicy
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, StrategyLocal> 
where Q: masa::Queue 
{
    type Hooks = LocalDeadlinePolicy<Self>;
}

// Map StrategyGlobal -> QueueGlobal
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, StrategyGlobal> 
where Q: masa::Queue 
{
    type Hooks = QueueGlobal<Self>;
}

// Map StrategyOldest -> PrioOldest
impl<Q, const E: bool> TonicPolicy for CompositePolicy<Q, E, StrategyOldest> 
where Q: masa::Queue 
{
    type Hooks = PrioOldest<Self>;
}
```

**Note**: Ensure `LocalDeadlinePolicy` and others are generic over `P: Policy` to respect `P::EARLY_RETURN`.

### Step 3: Application Utilities in `apps/app-utils`

In `apps/app-utils/src/config.rs`:

```rust
use structopt::StructOpt;
use masa::{QueueType, DeadlineStrategyType};

#[derive(StructOpt, Debug, Clone)]
pub struct PolicyArgs {
    #[structopt(long, default_value = "fifo")]
    pub queue: QueueType,

    #[structopt(long)]
    pub early_return: bool,

    #[structopt(long, default_value = "none")]
    pub deadline_strategy: DeadlineStrategyType,
}
```

In `apps/app-utils/src/lib.rs` (Macros):

```rust
#[macro_export]
macro_rules! launch_masa_server {
    ($policy_args:expr, $run_fn:ident, $app_args:expr) => {
        match $policy_args.queue {
            masa::QueueType::Fifo => {
                dispatch_early_return!(tokio::runtime::queue::FifoQueue, $policy_args, $run_fn, $app_args)
            }
            masa::QueueType::Prio => {
                dispatch_early_return!(tokio::runtime::queue::BinaryHeapQueue, $policy_args, $run_fn, $app_args)
            }
            masa::QueueType::PrioOldest => {
                dispatch_early_return!(tokio::runtime::queue::BinaryHeapRoundRobinQueue, $policy_args, $run_fn, $app_args)
            }
        }
    };
}

#[macro_export]
macro_rules! dispatch_early_return {
    ($Queue:ty, $policy_args:expr, $run_fn:ident, $app_args:expr) => {
        match $policy_args.early_return {
            true => { dispatch_strategy!($Queue, true, $policy_args, $run_fn, $app_args) }
            false => { dispatch_strategy!($Queue, false, $policy_args, $run_fn, $app_args) }
        }
    }
}

#[macro_export]
macro_rules! dispatch_strategy {
    ($Queue:ty, $Early:literal, $policy_args:expr, $run_fn:ident, $app_args:expr) => {
        match $policy_args.deadline_strategy {
            masa::DeadlineStrategyType::None => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::StrategyNone>>($app_args).await
            }
            masa::DeadlineStrategyType::Local => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::StrategyLocal>>($app_args).await
            }
            masa::DeadlineStrategyType::Global => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::StrategyGlobal>>($app_args).await
            }
            masa::DeadlineStrategyType::Oldest => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::StrategyOldest>>($app_args).await
            }
        }
    }
}
```

### Step 4: Application Usage

In `apps/hotel/src/reservation/main.rs`:

```rust
use structopt::StructOpt;
use tonic::transport::Server;
use app_utils::{launch_masa_server, config::PolicyArgs};
use tonic::masa::TonicPolicy;

#[derive(StructOpt, Debug, Clone)]
pub struct Args {
    #[structopt(flatten)]
    pub policy: PolicyArgs,
    
    #[structopt(short, long)]
    pub config: std::path::PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::from_args();
    // Macro handles the dispatch
    launch_masa_server!(args.policy, run_server, args)
}

// Generic entry point
async fn run_server<P: TonicPolicy>(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .policy::<P>()
        .build()?;

    rt.block_on(async {
        // Load config...
        
        Server::builder_with_policy::<P>()
            .add_service(ReservationServer::new(service))
            .serve_with_masa(addr)
            .await?;
        Ok(())
    })
}
```

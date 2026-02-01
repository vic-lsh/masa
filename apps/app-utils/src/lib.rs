pub mod load_gen;
pub mod logging;
pub mod pool;
pub mod stats;
pub mod timing;
pub mod config;

#[macro_export]
macro_rules! launch_masa_server {
    ($policy_args:expr, $run_fn:ident, $app_args:expr) => {
        match $policy_args.queue {
            masa::QueueType::Fifo => {
                $crate::dispatch_early_return!(masa::Fifo, $policy_args, $run_fn, $app_args)
            }
            masa::QueueType::Prio => {
                $crate::dispatch_early_return!(masa::Prio, $policy_args, $run_fn, $app_args)
            }
            masa::QueueType::PrioOldest => {
                $crate::dispatch_early_return!(masa::PrioOldest, $policy_args, $run_fn, $app_args)
            }
        }
    };
}

#[macro_export]
macro_rules! dispatch_early_return {
    ($Queue:ty, $policy_args:expr, $run_fn:ident, $app_args:expr) => {
        match $policy_args.early_return {
            true => { $crate::dispatch_policy!($Queue, true, $policy_args, $run_fn, $app_args) }
            false => { $crate::dispatch_policy!($Queue, false, $policy_args, $run_fn, $app_args) }
        }
    }
}

#[macro_export]
macro_rules! dispatch_policy {
    ($Queue:ty, $Early:literal, $policy_args:expr, $run_fn:ident, $app_args:expr) => {
        match $policy_args.deadline_policy {
            masa::DeadlinePolicyType::None => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyNone>>($app_args)
            }
            masa::DeadlinePolicyType::Local => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyLocal>>($app_args)
            }
            masa::DeadlinePolicyType::Global => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyGlobal>>($app_args)
            }
            masa::DeadlinePolicyType::Oldest => {
                $run_fn::<masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyOldest>>($app_args)
            }
        }
    }
}
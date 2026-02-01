pub mod load_gen;
pub mod logging;
pub mod pool;
pub mod stats;
pub mod timing;
pub mod config;

use tonic::masa::TonicPolicy;
use tonic::server::NamedService;
use tonic::http;

pub fn run_masa_server<P, Args, F, Fut, S, W, T>(
    args: Args,
    service_factory: F,
    wrapper: W,
) -> Result<(), Box<dyn std::error::Error>>
where
    P: TonicPolicy,
    F: FnOnce(Args) -> Fut,
    Fut: std::future::Future<Output = Result<(S, std::net::SocketAddr), Box<dyn std::error::Error>>>,
    W: FnOnce(S) -> T,
    T: NamedService + tower::Service<http::Request<tonic::transport::Body>, Response = http::Response<tonic::body::BoxBody>, Error = std::convert::Infallible> + Clone + Send + 'static,
    T::Future: Send + 'static,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .policy::<P>()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let (inner_service, addr) = service_factory(args).await?;
        let service = wrapper(inner_service);

        tonic::transport::Server::builder_with_policy::<P>()
            .add_service(service)
            .serve_with_masa(addr)
            .await?;

        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}

#[macro_export]
macro_rules! launch_masa_server {
    ($ServerStruct:ident, $policy_args:expr, $service_factory:expr, $app_args:expr) => {
        match $policy_args.queue {
            masa::QueueType::Fifo => {
                $crate::dispatch_early_return!($ServerStruct, masa::Fifo, $policy_args, $service_factory, $app_args)
            }
            masa::QueueType::Prio => {
                $crate::dispatch_early_return!($ServerStruct, masa::Prio, $policy_args, $service_factory, $app_args)
            }
            masa::QueueType::PrioOldest => {
                $crate::dispatch_early_return!($ServerStruct, masa::PrioOldest, $policy_args, $service_factory, $app_args)
            }
        }
    };
}

#[macro_export]
macro_rules! dispatch_early_return {
    ($ServerStruct:ident, $Queue:ty, $policy_args:expr, $service_factory:expr, $app_args:expr) => {
        match $policy_args.early_return {
            true => { $crate::dispatch_policy!($ServerStruct, $Queue, masa::EarlyReturnEnabled, $policy_args, $service_factory, $app_args) }
            false => { $crate::dispatch_policy!($ServerStruct, $Queue, masa::EarlyReturnDisabled, $policy_args, $service_factory, $app_args) }
        }
    }
}

#[macro_export]
macro_rules! dispatch_policy {
    ($ServerStruct:ident, $Queue:ty, $Early:ty, $policy_args:expr, $service_factory:expr, $app_args:expr) => {
        match $policy_args.deadline_policy {
            masa::DeadlinePolicyType::None => {
                type P = masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyNone>;
                $crate::run_masa_server::<
                    P,
                    _, _, _, _, _, _
                >(
                    $app_args,
                    $service_factory,
                    |s| $ServerStruct::<_, <P as tonic::masa::TonicPolicy>::Hooks>::with_custom_context(s)
                )
            }
            masa::DeadlinePolicyType::Local => {
                type P = masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyLocal>;
                $crate::run_masa_server::<
                    P,
                    _, _, _, _, _, _
                >(
                    $app_args,
                    $service_factory,
                    |s| $ServerStruct::<_, <P as tonic::masa::TonicPolicy>::Hooks>::with_custom_context(s)
                )
            }
            masa::DeadlinePolicyType::Global => {
                type P = masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyGlobal>;
                $crate::run_masa_server::<
                    P,
                    _, _, _, _, _, _
                >(
                    $app_args,
                    $service_factory,
                    |s| $ServerStruct::<_, <P as tonic::masa::TonicPolicy>::Hooks>::with_custom_context(s)
                )
            }
            masa::DeadlinePolicyType::Oldest => {
                type P = masa::CompositePolicy<$Queue, $Early, masa::DeadlinePolicyOldest>;
                $crate::run_masa_server::<
                    P,
                    _, _, _, _, _, _
                >(
                    $app_args,
                    $service_factory,
                    |s| $ServerStruct::<_, <P as tonic::masa::TonicPolicy>::Hooks>::with_custom_context(s)
                )
            }
        }
    }
}
use std::{
    marker::PhantomData,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use hyper::rt::{Exec, Executor};
use masa_integration_tests::pb::{
    child_service_client::ChildServiceClient,
    child_service_server::{ChildService, ChildServiceServer},
    parent_service_client::*,
    parent_service_server::*,
    Input1, Input2, Output1, Output2,
};
use tonic::{masa::PrioritySelector, transport::Server, GrpcMethod, Request, Response, Status};
use tonic_masa::PriorityHint;

struct ParentSvc {
    child_addr: &'static str,
    fanout_factor: usize,
}

impl ParentSvc {
    fn new(child_addr: &'static str, fanout_factor: usize) -> Self {
        Self {
            child_addr,
            fanout_factor,
        }
    }
}

#[tonic::async_trait]
impl<P> ParentService for ParentSvc
where
    P: PrioritySelector,
    P::ClientContext: Send + Sync + 'static,
    P::ParentContext: 'static,
{
    async fn rpc(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
        let mut client = ChildServiceClient::<_, P>::connect_with_custom_context(format!(
            "http://{}",
            self.child_addr
        ))
        .await
        .unwrap();

        for _ in 0..self.fanout_factor {
            println!("calling child...");
            client.rpc1(Request::new(Input1 {})).await.unwrap();
        }

        Ok(Response::new(Output1 {}))
    }

    async fn fanout_rpc(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
        let client = ChildServiceClient::<_, P>::connect_with_custom_context(format!(
            "http://{}",
            self.child_addr
        ))
        .await
        .unwrap();

        let mut handles: Vec<_> = Vec::new();
        for _ in 0..self.fanout_factor {
            let mut c = client.clone();
            handles.push(async_executor::spawn(async move {
                c.rpc1(Request::new(Input1 {})).await.unwrap();
            }));
        }
        for h in handles {
            h.await;
        }

        Ok(Response::new(Output1 {}))
    }
}

struct ChildSvc;

#[tonic::async_trait]
impl ChildService for ChildSvc {
    async fn rpc1(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
        Ok(Response::new(Output1 {}))
    }

    async fn rpc2(&self, _req: Request<Input2>) -> Result<Response<Output2>, Status> {
        Ok(Response::new(Output2 {}))
    }
}

struct ExecImpl;

impl<F> Executor<F> for ExecImpl
where
    F: std::future::Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, fut: F, prio: PriorityHint) {
        async_executor::spawn_with_prio(fut, prio)
            .fallible()
            .detach();
    }
}

struct MockServerCtx;

impl ServerHooks for MockServerCtx {
    fn new(_service_name: &'static str) -> Self {
        Self
    }
}

struct MockParentCtx {}

impl<C: ClientHooks, S: ServerHooks> ParentHooks<C, S> for MockParentCtx {
    fn begin<B>(_method: GrpcMethod, _req: &http::Request<B>, _server_ctx: Arc<S>) -> Self {
        Self {}
    }
}

struct MockChildCtx;

impl ClientHooks for MockChildCtx {
    fn new<T>(_method: GrpcMethod, _req: &Request<T>) -> Self {
        Self {}
    }
}

async fn make_parent_child_svcs<P>(
    parent_svc_addr: &'static str,
    child_svc_addr: &'static str,
    fanout_factor: usize,
) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>)
where
    P: PrioritySelector,
    P::ClientContext: Send + Sync + 'static,
    P::ParentContext: 'static,
{
    let child_svc = tokio::spawn(async {
        Server::builder()
            .add_service(ChildServiceServer::<_, P>::with_custom_context(ChildSvc))
            .serve_with_executor(
                child_svc_addr.parse().unwrap(),
                Exec::Executor(Arc::new(ExecImpl)),
            )
            .await
            .unwrap();
    });

    let parent_svc = tokio::spawn(async move {
        Server::builder()
            .add_service(ParentServiceServer::<_, P>::with_custom_context(
                ParentSvc::<P>::new(child_svc_addr, fanout_factor),
            ))
            .serve_with_executor(
                parent_svc_addr.parse().unwrap(),
                Exec::Executor(Arc::new(ExecImpl)),
            )
            .await
            .unwrap();
    });

    (parent_svc, child_svc)
}

#[tokio::test]
async fn test_service_ctx_construction() {
    static N_SERVICE_CTX_CTORS: AtomicUsize = AtomicUsize::new(0);

    struct TestCtorCountServerCtx;

    impl ServerHooks for TestCtorCountServerCtx {
        fn new(_service_name: &'static str) -> Self {
            N_SERVICE_CTX_CTORS.fetch_add(1, Ordering::Relaxed);
            Self
        }
    }

    let n_svcs = 12;
    let _handles: Vec<_> = (0..n_svcs)
        .map(|i| {
            let addr = format!("127.0.0.1:{}", 7878 + i);
            tokio::spawn(async move {
                Server::builder()
                    .add_service(ChildServiceServer::<
                        _,
                        TestCtorCountServerCtx,
                        MockChildCtx,
                        MockParentCtx,
                    >::with_custom_context(ChildSvc))
                    .serve_with_executor(addr.parse().unwrap(), Exec::Executor(Arc::new(ExecImpl)))
                    .await
                    .unwrap();
            });
        })
        .collect();

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(N_SERVICE_CTX_CTORS.load(Ordering::Relaxed), n_svcs);
}

#[tokio::test]
async fn test_child_rpc_hooks_invocations() {
    static N_BEFORE_CHILD_RPCS: AtomicUsize = AtomicUsize::new(0);
    static N_AFTER_CHILD_RPCS: AtomicUsize = AtomicUsize::new(0);

    struct TestChildRpcParentCtx {}

    impl<C: ClientHooks, S: ServerHooks> ParentHooks<C, S> for TestChildRpcParentCtx {
        fn begin<B>(_method: GrpcMethod, _req: &http::Request<B>, _server_ctx: Arc<S>) -> Self {
            Self {}
        }

        fn before_child_rpc<T>(
            &self,
            _method: GrpcMethod,
            _req: &mut Request<T>,
            _child_ctx: &mut C,
        ) -> Option<Status> {
            N_BEFORE_CHILD_RPCS.fetch_add(1, Ordering::Relaxed);
            None
        }

        fn after_child_rpc<T>(
            &self,
            _method: GrpcMethod,
            _resp: &mut Result<Response<T>, Status>,
            _child_ctx: C,
        ) -> Option<Status> {
            N_AFTER_CHILD_RPCS.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    let parent_svc_addr = "127.0.0.1:4455";
    let child_svc_addr = "127.0.0.1:4466";
    let fanout_factor = 10;
    let (_parent, _child) = make_parent_child_svcs::<
        MockServerCtx,
        MockChildCtx,
        TestChildRpcParentCtx,
    >(parent_svc_addr, child_svc_addr, fanout_factor)
    .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut parent_cl = ParentServiceClient::connect(format!("http://{}", parent_svc_addr))
        .await
        .unwrap();

    parent_cl.rpc(Request::new(Input1 {})).await.unwrap();

    assert_eq!(N_BEFORE_CHILD_RPCS.load(Ordering::Relaxed), fanout_factor);
    assert_eq!(N_AFTER_CHILD_RPCS.load(Ordering::Relaxed), fanout_factor);
    N_BEFORE_CHILD_RPCS.store(0, Ordering::Relaxed);
    N_AFTER_CHILD_RPCS.store(0, Ordering::Relaxed);

    parent_cl.fanout_rpc(Request::new(Input1 {})).await.unwrap();

    assert_eq!(N_BEFORE_CHILD_RPCS.load(Ordering::Relaxed), fanout_factor);
    assert_eq!(N_AFTER_CHILD_RPCS.load(Ordering::Relaxed), fanout_factor);
    N_BEFORE_CHILD_RPCS.store(0, Ordering::Relaxed);
    N_AFTER_CHILD_RPCS.store(0, Ordering::Relaxed);
}

#[tokio::test]
async fn test_child_ctx_hook_invocations() {
    static N_BEFORE_SEND: AtomicUsize = AtomicUsize::new(0);
    static N_AFTER_RECV: AtomicUsize = AtomicUsize::new(0);

    struct TestInvocationChildCtx;

    impl ClientHooks for TestInvocationChildCtx {
        fn new<T>(_method: GrpcMethod, _req: &Request<T>) -> Self {
            Self {}
        }

        fn before_send<T>(&mut self, _req: &mut Request<T>) {
            N_BEFORE_SEND.fetch_add(1, Ordering::Relaxed);
        }

        fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {
            N_AFTER_RECV.fetch_add(1, Ordering::Relaxed);
        }
    }

    let parent_svc_addr = "127.0.0.1:4355";
    let child_svc_addr = "127.0.0.1:4366";
    let fanout_factor = 10;

    let (_parent, _child) = make_parent_child_svcs::<
        MockServerCtx,
        TestInvocationChildCtx,
        MockParentCtx,
    >(parent_svc_addr, child_svc_addr, fanout_factor)
    .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut parent_cl = ParentServiceClient::connect(format!("http://{}", parent_svc_addr))
        .await
        .unwrap();

    parent_cl.rpc(Request::new(Input1 {})).await.unwrap();

    assert_eq!(N_BEFORE_SEND.load(Ordering::Relaxed), fanout_factor);
    assert_eq!(N_AFTER_RECV.load(Ordering::Relaxed), fanout_factor);
    N_BEFORE_SEND.store(0, Ordering::Relaxed);
    N_AFTER_RECV.store(0, Ordering::Relaxed);

    parent_cl.fanout_rpc(Request::new(Input1 {})).await.unwrap();

    assert_eq!(N_BEFORE_SEND.load(Ordering::Relaxed), fanout_factor);
    assert_eq!(N_AFTER_RECV.load(Ordering::Relaxed), fanout_factor);
    N_BEFORE_SEND.store(0, Ordering::Relaxed);
    N_AFTER_RECV.store(0, Ordering::Relaxed);
}

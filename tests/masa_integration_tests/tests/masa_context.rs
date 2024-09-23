use std::{
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
use tonic::{
    body::BoxBody,
    masa::{ClientStubHooks, RequestHandlerHooks, ServerContext},
    transport::Server,
    GrpcMethod, Request, Response, Status,
};
use tonic_masa::DeadlineHint;

static N_CHILD_RPCS: AtomicUsize = AtomicUsize::new(0);

struct ParentSvc {
    child_addr: &'static str,
}

#[tonic::async_trait]
impl ParentService for ParentSvc {
    async fn rpc(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
        let mut client =
            ChildServiceClient::<_, TestChildCtx, TestParentCtx>::connect_with_custom_context(
                format!("http://{}", self.child_addr),
            )
            .await
            .unwrap();

        let fanout_factor = 10;
        for _ in 0..fanout_factor {
            client.rpc1(Request::new(Input1 {})).await.unwrap();
        }

        Ok(Response::new(Output1 {}))
    }

    async fn fanout_rpc(&self, _req: Request<Input1>) -> Result<Response<Output1>, Status> {
        let mut client =
            ChildServiceClient::<_, TestChildCtx, TestParentCtx>::connect_with_custom_context(
                format!("http://{}", self.child_addr),
            )
            .await
            .unwrap();

        let fanout_factor = 10;
        let mut handles: Vec<_> = Vec::new();
        for _ in 0..fanout_factor {
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
    fn execute(&self, fut: F, ddl: DeadlineHint) {
        async_executor::spawn_with_ddl(fut, ddl).fallible().detach();
    }
}

struct TestParentCtx {}

impl<C: ClientStubHooks> RequestHandlerHooks<C> for TestParentCtx {
    fn begin<B>(
        method: GrpcMethod,
        req: &http::Request<B>,
        server_ctx: Arc<ServerContext>,
    ) -> Self {
        Self {}
    }

    fn before_child_rpc<T>(&self, method: GrpcMethod, _req: &mut Request<T>, _child_ctx: &mut C) {
        N_CHILD_RPCS.fetch_add(1, Ordering::Relaxed);
    }

    fn after_child_rpc<T>(
        &self,
        method: GrpcMethod,
        _resp: &mut Result<Response<T>, Status>,
        _child_ctx: C,
    ) {
    }

    fn before_poll(&self) {}

    fn finalize(&self, response: &mut http::Response<BoxBody>) {}
}

struct TestChildCtx;

impl ClientStubHooks for TestChildCtx {
    fn new<T>(method: GrpcMethod, _req: &Request<T>) -> Self {
        Self {}
    }

    fn before_send<T>(&mut self, _req: &mut Request<T>) {}

    fn after_recv<T>(&mut self, _response: &mut Result<Response<T>, Status>) {}
}

#[tokio::test]
async fn test_ctx_hook() {
    let parent_svc_addr = "127.0.0.1:4455";
    let child_svc_addr = "127.0.0.1:4466";

    let child_svc = tokio::spawn(async {
        Server::builder()
            .add_service(
                ChildServiceServer::<_, TestChildCtx, TestParentCtx>::with_custom_context(ChildSvc),
            )
            .serve_with_executor(
                child_svc_addr.parse().unwrap(),
                Exec::Executor(Arc::new(ExecImpl)),
            )
            .await
            .unwrap();
    });

    let parent_svc = tokio::spawn(async {
        Server::builder()
            .add_service(
                ParentServiceServer::<_, TestChildCtx, TestParentCtx>::with_custom_context(
                    ParentSvc {
                        child_addr: child_svc_addr,
                    },
                ),
            )
            .serve_with_executor(
                parent_svc_addr.parse().unwrap(),
                Exec::Executor(Arc::new(ExecImpl)),
            )
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut parent_cl = ParentServiceClient::connect(format!("http://{}", parent_svc_addr))
        .await
        .unwrap();

    parent_cl.rpc(Request::new(Input1 {})).await.unwrap();

    assert_eq!(N_CHILD_RPCS.load(Ordering::Relaxed), 10);
    N_CHILD_RPCS.store(0, Ordering::Relaxed);

    parent_cl.fanout_rpc(Request::new(Input1 {})).await.unwrap();

    assert_eq!(N_CHILD_RPCS.load(Ordering::Relaxed), 10);
    N_CHILD_RPCS.store(0, Ordering::Relaxed);
}

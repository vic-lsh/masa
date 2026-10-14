//! Minimal real-transport example: both paper policies, no Masa budget module.
pub type PaperStack =
    rpcstack::policy_stack![tailclipper::TailClipperModule, rajomon::PaperRajomonModule];
pub type PaperHooks = rpcstack_tonic::PolicyHooks<PaperStack>;
pub mod pb {
    tonic::include_proto!("paper");
}
use pb::{echo_client::EchoClient, echo_server::Echo, Input, Output};
use rpcstack_tonic::RequestExt;
use tonic::{Request, Response, Status};

/// A leaf, or a frontend that calls the next hop with generated stubs.
#[derive(Clone, Default)]
pub struct EchoService {
    pub downstream: Option<EchoClient<tonic::transport::Channel>>,
}
#[tonic::async_trait]
impl Echo for EchoService {
    async fn call(&self, request: Request<Input>) -> Result<Response<Output>, Status> {
        if let Some(mut client) = self.downstream.clone() {
            let mut output = client.call(Input {}).await?.into_inner();
            output.hops += 1;
            return Ok(Response::new(output));
        }
        let arrival = request
            .get_wire::<tailclipper::TailClipperModule>()
            .ok_or_else(|| Status::internal("missing arrival"))?;
        let tokens = request
            .get_wire::<rajomon::PaperRajomonModule>()
            .ok_or_else(|| Status::internal("missing tokens"))?;
        Ok(Response::new(Output {
            arrival_ns: arrival.arrival_ns,
            tokens: tokens.tokens,
            hops: 1,
        }))
    }
}

/// Bind before spawning so clients cannot race server startup or port reuse.
pub async fn serve(
    service: EchoService,
) -> Result<(String, tokio::task::JoinHandle<()>), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(pb::echo_server::EchoServer::new(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .expect("serve RPC");
    });
    Ok((endpoint, task))
}

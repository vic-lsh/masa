use paper_rpc::{
    pb::{echo_client::EchoClient, Input},
    serve, EchoService,
};
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (leaf, leaf_task) = serve(EchoService::default()).await?;
    let downstream = EchoClient::connect(leaf).await?;
    let (front, front_task) = serve(EchoService {
        downstream: Some(downstream),
    })
    .await?;
    let mut client = EchoClient::connect(front).await?;
    let output = client.call(Input {}).await?.into_inner();
    println!(
        "hops={} arrival_ns={} tokens={}",
        output.hops, output.arrival_ns, output.tokens
    );
    front_task.abort();
    leaf_task.abort();
    Ok(())
}

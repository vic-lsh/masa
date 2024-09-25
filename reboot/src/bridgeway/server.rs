pub mod bridge {
    tonic::include_proto!("bridge");
}
mod common;
mod exec;
mod graph;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use env_logger::{Builder, Env};
use structopt::StructOpt;

use hyper::rt::Exec;
use tokio::task::JoinHandle;
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};
use tonic_masa::{
    Address, Context, GlobalGraph, Latency, LocalGraph, LocalGraphTracker, Path, EST_ONLINE,
    QUEUE_EDF,
};

use bridge::{
    worker_client::WorkerClient,
    worker_server::{Worker, WorkerServer},
    HelloReply, HelloRequest,
};
use common::{busy_spin, time_now, VirtualServer};
use exec::ExecImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub n_hops: usize,
    #[structopt(short, long, required = true)]
    pub n_threads: usize,
}

pub struct WorkerImpl {
    local_graphs: HashMap<Path, LocalGraph>,
    local_graph_trackers: HashMap<Path, RwLock<LocalGraphTracker>>,
    clients: HashMap<Path, WorkerClient<Channel>>,
}

impl WorkerImpl {
    pub fn new(
        local_graphs: HashMap<Path, LocalGraph>,
        clients: HashMap<Path, WorkerClient<Channel>>,
    ) -> Self {
        let local_graph_trackers = local_graphs
            .iter()
            .map(|(path, local_graph)| {
                let local_graph = LocalGraphTracker::from(local_graph.clone());
                (path.clone(), RwLock::new(local_graph))
            })
            .collect();
        Self {
            local_graphs,
            local_graph_trackers,
            clients,
        }
    }

    fn set_child_ctx(&self, ctx: &Context, request: &mut Request<HelloRequest>, path: &Path) {
        let mut deadline = ctx.deadline();
        if QUEUE_EDF {
            let graph = self
                .local_graph_trackers
                .get(ctx.graph_id())
                .unwrap()
                .read()
                .unwrap();
            deadline = ctx.deadline() - graph.estimate_suffix(path);
        }
        let child_ctx = Context::new(
            ctx.graph_id().clone(),
            ctx.request_id(),
            deadline,
            time_now(),
        );
        request.metadata_mut().insert_ctx("ctx", &child_ctx);
    }

    fn track_span(&self, ctx: &Context, path: &Path, latency: Latency) {
        if EST_ONLINE {
            let mut graph = self
                .local_graph_trackers
                .get(ctx.graph_id())
                .unwrap()
                .write()
                .unwrap();
            graph.track_span(path, latency);
        }
    }
}

#[tonic::async_trait]
impl Worker for WorkerImpl {
    // [TODO] Initialize servers in different processes.

    async fn say_hello_i4(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let ctx = request.metadata().get_ctx("ctx").unwrap();
        let graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);

        let spans = graph.spans();
        assert!(spans.len() == 3);

        for i in 0..spans.len() {
            let span = &spans[i];
            let path = span.path();

            if i == 0 || i == spans.len() - 1 {
                let elapse = span.distribution().sample(ctx.request_id());
                let start_at = time_now();
                busy_spin(Duration::from_micros(elapse));
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            } else {
                let mut client = self.clients.get(path).unwrap().clone();
                let mut request = Request::new(HelloRequest {
                    name: "SayHelloI3".to_string(),
                });

                self.set_child_ctx(&ctx, &mut request, path);
                let start_at = time_now();
                client.say_hello_i3(request).await.unwrap();
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            }
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_hello_i3(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let ctx = request.metadata().get_ctx("ctx").unwrap();
        let graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);

        let spans = graph.spans();
        assert!(spans.len() == 3);

        for i in 0..spans.len() {
            let span = &spans[i];
            let path = span.path();

            if i == 0 || i == spans.len() - 1 {
                let elapse = span.distribution().sample(ctx.request_id());
                let start_at = time_now();
                busy_spin(Duration::from_micros(elapse));
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            } else {
                let mut client = self.clients.get(path).unwrap().clone();
                let mut request = Request::new(HelloRequest {
                    name: "SayHelloI2".to_string(),
                });

                self.set_child_ctx(&ctx, &mut request, path);
                let start_at = time_now();
                client.say_hello_i2(request).await.unwrap();
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            }
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_hello_i2(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let ctx = request.metadata().get_ctx("ctx").unwrap();
        let graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);

        let spans = graph.spans();
        assert!(spans.len() == 3);

        for i in 0..spans.len() {
            let span = &spans[i];
            let path = span.path();

            if i == 0 || i == spans.len() - 1 {
                let elapse = span.distribution().sample(ctx.request_id());
                let start_at = time_now();
                busy_spin(Duration::from_micros(elapse));
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            } else {
                let mut client = self.clients.get(path).unwrap().clone();
                let mut request = Request::new(HelloRequest {
                    name: "SayHelloI1".to_string(),
                });

                self.set_child_ctx(&ctx, &mut request, path);
                let start_at = time_now();
                client.say_hello_i1(request).await.unwrap();
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            }
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_hello_i1(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let ctx = request.metadata().get_ctx("ctx").unwrap();
        let graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);

        let spans = graph.spans();
        assert!(spans.len() == 2);

        for i in 0..spans.len() {
            let span = &spans[i];
            let path = span.path();

            if i == 0 || i == spans.len() - 1 {
                let elapse = span.distribution().sample(ctx.request_id());
                let start_at = time_now();
                busy_spin(Duration::from_micros(elapse));
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            } else {
                let mut client = self.clients.get(path).unwrap().clone();
                let mut request = Request::new(HelloRequest {
                    name: "SayHelloI1".to_string(),
                });

                self.set_child_ctx(&ctx, &mut request, path);
                let start_at = time_now();
                client.say_hello_i1(request).await.unwrap();
                let latency = time_now() - start_at;
                self.track_span(&ctx, path, latency);
            }
        }

        let reply = HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

fn get_global_graph(args: Args) -> GlobalGraph {
    assert!(args.n_hops > 0);
    assert!(args.n_hops <= 4);

    let global_graph = {
        if args.n_hops == 1 {
            graph::get_global_graph_i1()
        } else if args.n_hops == 2 {
            graph::get_global_graph_i2()
        } else if args.n_hops == 4 {
            graph::get_global_graph_i4()
        } else {
            panic!("Unsupported n_hops: {}", args.n_hops);
        }
    };
    global_graph
}

fn get_servers(args: Args, global_graph: GlobalGraph) -> Vec<VirtualServer> {
    let mut servers = Vec::new();

    if args.n_hops >= 1 {
        let server1 = {
            let addr: Address = "[::1]:50051".to_string();
            let conn_addrs = HashMap::new();
            let path: Path = "/bridge.Worker/SayHelloI1".to_string();
            let local_graphs = {
                let mut graphs = HashMap::new();
                graphs.insert(
                    global_graph.graph_id().clone(),
                    global_graph.get_local_graph(&path).clone(),
                );
                graphs
            };
            let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads, false);
            server
        };
        servers.push(server1);
    }

    if args.n_hops >= 2 {
        let server2 = {
            let addr: Address = "[::1]:50052".to_string();
            let mut conn_addrs = HashMap::new();
            conn_addrs.insert(
                "/bridge.Worker/SayHelloI1".to_string() as Path,
                "http://[::1]:50051".to_string() as Address,
            );
            let path: Path = "/bridge.Worker/SayHelloI2".to_string();
            let local_graphs = {
                let mut graphs = HashMap::new();
                graphs.insert(
                    global_graph.graph_id().clone(),
                    global_graph.get_local_graph(&path).clone(),
                );
                graphs
            };
            let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads, false);
            server
        };
        servers.push(server2);
    }

    if args.n_hops >= 3 {
        let server3 = {
            let addr: Address = "[::1]:50053".to_string();
            let mut conn_addrs = HashMap::new();
            conn_addrs.insert(
                "/bridge.Worker/SayHelloI2".to_string() as Path,
                "http://[::1]:50052".to_string() as Address,
            );
            let path: Path = "/bridge.Worker/SayHelloI3".to_string();
            let local_graphs = {
                let mut graphs = HashMap::new();
                graphs.insert(
                    global_graph.graph_id().clone(),
                    global_graph.get_local_graph(&path).clone(),
                );
                graphs
            };
            let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads, false);
            server
        };
        servers.push(server3);
    }

    if args.n_hops >= 4 {
        let server4 = {
            let addr: Address = "[::1]:50054".to_string();
            let mut conn_addrs = HashMap::new();
            conn_addrs.insert(
                "/bridge.Worker/SayHelloI3".to_string() as Path,
                "http://[::1]:50053".to_string() as Address,
            );
            let path: Path = "/bridge.Worker/SayHelloI4".to_string();
            let local_graphs = {
                let mut graphs = HashMap::new();
                graphs.insert(
                    global_graph.graph_id().clone(),
                    global_graph.get_local_graph(&path).clone(),
                );
                graphs
            };
            let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads, false);
            server
        };
        servers.push(server4);
    }

    servers
}

async fn get_clients(server: &VirtualServer) -> HashMap<Path, WorkerClient<Channel>> {
    let mut clients = HashMap::new();
    for (path, addr) in server.conn_addrs().iter() {
        let client = WorkerClient::connect(addr.clone()).await.unwrap();
        clients.insert(path.clone(), client);
    }
    clients
}

async fn start_servers(servers: Vec<VirtualServer>) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();

    for i in 0..servers.len() {
        let server = servers[i].clone();

        let ex = Arc::new(ExecImpl::new(Arc::new(smol::Executor::new())));
        for _ in 0..server.n_threads() {
            let ex = ex.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(ex.run());
            });
        }

        let h = tokio::spawn(async move {
            let addr = server.addr().parse().unwrap();
            let local_graphs = server.local_graphs().clone();
            let clients = get_clients(&server).await;

            let worker = WorkerImpl::new(local_graphs, clients);
            log::warn!("Listening on {}...", addr);
            Server::builder()
                .add_service(WorkerServer::new(worker))
                .serve_with_executor(addr, Exec::Executor(ex))
                .await
                .unwrap();
        });
        handles.push(h);

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    handles
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let global_graph = get_global_graph(args.clone());
    let servers = get_servers(args.clone(), global_graph.clone());
    let handles = start_servers(servers).await;
    for h in handles {
        h.await.unwrap();
    }

    Ok(())
}

fn init_logging() {
    Builder::from_env(Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{} [{}:{}] {}",
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                // record.target(),
                record.args()
            )
        })
        .init();
    log::info!("Logging initialized");
}

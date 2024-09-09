pub mod bridge {
    tonic::include_proto!("bridge");
}
mod common;
mod exec;
mod graph;
mod manager;

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use env_logger::{Builder, Env};
use structopt::StructOpt;

use hyper::rt::Exec;
use tokio::task::JoinHandle;
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};
use tonic_masa::{Address, GlobalGraph, LocalGraph, Path};

use bridge::{
    tung_chung_client::TungChungClient,
    tung_chung_server::{TungChung, TungChungServer},
    WalkReply, WalkRequest,
};
use common::{busy_spin, time_now, VirtualServer};
use exec::ExecImpl;
use manager::{Hotel, Manager};

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Server for benchmarking")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub n_threads: usize,
}

pub struct TungChungImpl {
    local_graphs: HashMap<Path, LocalGraph>,
    clients: HashMap<Path, TungChungClient<Channel>>,
    manager: Option<Manager>,
}

impl TungChungImpl {
    pub fn new(
        local_graphs: HashMap<Path, LocalGraph>,
        clients: HashMap<Path, TungChungClient<Channel>>,
    ) -> Self {
        Self {
            local_graphs,
            clients,
            manager: None,
        }
    }
}

impl TungChungImpl {
    async fn start_manager(&mut self) -> Result<(), Box<dyn Error>> {
        // [TODO] Pass args.
        let n_hotels = 10_000;
        let payload = 16;
        let cache_addr = "memcache://127.0.0.1:11003".to_string();
        let cache_conn = 32;
        let cache_miss_rate = 0.5;
        let db_addr = "mongodb://127.0.0.1:27003".to_string();

        let manager = Manager::new(
            n_hotels,
            payload,
            cache_addr,
            cache_conn,
            cache_miss_rate,
            db_addr,
        )
        .await?;
        self.manager = Some(manager);

        Ok(())
    }

    async fn fetch_mixture(&self) -> Vec<Hotel> {
        // [TODO] Make reproducible.
        let manager = self.manager.as_ref().unwrap();
        let mut names = Vec::new();
        for i in 0..3 {
            names.push(format!("Tung Chung Ave {}", 3667 + i));
        }
        let hotels = manager.fetch_mixture(names).await;
        log::warn!("hotels: {:?}", hotels);
        hotels
    }
}

#[tonic::async_trait]
impl TungChung for TungChungImpl {
    // [TODO] Initialize servers in different processes.

    async fn say_frontend(
        &self,
        request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 4);

        assert!(spans.first().unwrap().path() == "Head");
        let elapse = spans.first().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        for span in spans.iter().skip(1).take(spans.len() - 2) {
            let mut client = self.clients.get(span.path()).unwrap().clone();
            let mut request = Request::new(WalkRequest {
                name: "SayTungChung".to_string(),
            });
            request.metadata_mut().insert_ctx("par_ctx", &ctx);
            if span.path() == "/bridge.TungChung/SaySearch" {
                client.say_search(request).await.unwrap();
            } else if span.path() == "/bridge.TungChung/SayProfile" {
                client.say_profile(request).await.unwrap();
            } else {
                panic!("Invalid path: {:?}", span.path());
            }
        }

        assert!(spans.last().unwrap().path() == "Tail");
        let elapse = spans.last().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_frontend,{},{},{}",
                ctx.request_id(),
                latency_spin,
                latency
            );
        }

        self.fetch_mixture().await;

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_search(
        &self,
        request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 4);

        assert!(spans.first().unwrap().path() == "Head");
        let elapse = spans.first().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        for span in spans.iter().skip(1).take(spans.len() - 2) {
            let mut client = self.clients.get(span.path()).unwrap().clone();
            let mut request = Request::new(WalkRequest {
                name: "SayTungChung".to_string(),
            });
            request.metadata_mut().insert_ctx("par_ctx", &ctx);
            if span.path() == "/bridge.TungChung/SayGeo" {
                client.say_geo(request).await.unwrap();
            } else if span.path() == "/bridge.TungChung/SayRate" {
                client.say_rate(request).await.unwrap();
            } else {
                panic!("Invalid path: {:?}", span.path());
            }
        }

        assert!(spans.last().unwrap().path() == "Tail");
        let elapse = spans.last().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_search,{},{},{}",
                ctx.request_id(),
                latency_spin,
                latency
            );
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_profile(
        &self,
        request: Request<WalkRequest>,
    ) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 2);

        assert!(spans.first().unwrap().path() == "Head");
        let elapse = spans.first().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        assert!(spans.last().unwrap().path() == "Tail");
        let elapse = spans.last().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!(
                "say_profile,{},{},{}",
                ctx.request_id(),
                latency_spin,
                latency
            );
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_geo(&self, request: Request<WalkRequest>) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 2);

        assert!(spans.first().unwrap().path() == "Head");
        let elapse = spans.first().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        assert!(spans.last().unwrap().path() == "Tail");
        let elapse = spans.last().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!("say_geo,{},{},{}", ctx.request_id(), latency_spin, latency);
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }

    async fn say_rate(&self, request: Request<WalkRequest>) -> Result<Response<WalkReply>, Status> {
        let start_at = time_now();
        let mut latency_spin = 0;

        let mut ctx = request.metadata().get_ctx("ctx").unwrap();
        let local_graph = self.local_graphs.get(ctx.graph_id()).unwrap();
        log::info!("ctx: {:?}", ctx);
        ctx.set_local_graph(local_graph.clone());

        let spans = local_graph.spans();
        assert!(spans.len() == 2);

        assert!(spans.first().unwrap().path() == "Head");
        let elapse = spans.first().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        assert!(spans.last().unwrap().path() == "Tail");
        let elapse = spans.last().unwrap().get_distribution().estimate();
        // .sample(ctx.request_id());
        busy_spin(Duration::from_micros(elapse));
        latency_spin += elapse;

        let finish_at = time_now();
        let latency = finish_at - start_at;

        // [OPTION] Log by probability.
        // if ctx.request_id() % 10 == 0 {
        if true {
            log::warn!("say_rate,{},{},{}", ctx.request_id(), latency_spin, latency);
        }

        let reply = WalkReply {
            message: format!("Walk {}!", request.into_inner().name),
        };
        Ok(Response::new(reply))
    }
}

fn get_servers(args: Args, global_graph: GlobalGraph) -> Vec<VirtualServer> {
    let mut servers = Vec::new();

    let server_frontend = {
        let addr: Address = "[::1]:50051".to_string();
        let mut conn_addrs = HashMap::new();
        conn_addrs.insert(
            "/bridge.TungChung/SaySearch".to_string() as Path,
            "http://[::1]:50052".to_string() as Address,
        );
        conn_addrs.insert(
            "/bridge.TungChung/SayProfile".to_string() as Path,
            "http://[::1]:50053".to_string() as Address,
        );
        let path: Path = "/bridge.TungChung/SayFrontend".to_string();
        let local_graphs = {
            let mut graphs = HashMap::new();
            graphs.insert(
                global_graph.graph_id().clone(),
                global_graph.get_local_graph(&path).clone(),
            );
            graphs
        };
        let server = VirtualServer::new(addr, conn_addrs, local_graphs, args.n_threads, true);
        server
    };
    servers.push(server_frontend);

    let server_search = {
        let addr: Address = "[::1]:50052".to_string();
        let mut conn_addrs = HashMap::new();
        conn_addrs.insert(
            "/bridge.TungChung/SayGeo".to_string() as Path,
            "http://[::1]:50054".to_string() as Address,
        );
        conn_addrs.insert(
            "/bridge.TungChung/SayRate".to_string() as Path,
            "http://[::1]:50055".to_string() as Address,
        );
        let path: Path = "/bridge.TungChung/SaySearch".to_string();
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
    servers.push(server_search);

    let server_profile = {
        let addr: Address = "[::1]:50053".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/bridge.TungChung/SayProfile".to_string();
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
    servers.push(server_profile);

    let server_geo = {
        let addr: Address = "[::1]:50054".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/bridge.TungChung/SayGeo".to_string();
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
    servers.push(server_geo);

    let server_rate = {
        let addr: Address = "[::1]:50055".to_string();
        let conn_addrs = HashMap::new();
        let path: Path = "/bridge.TungChung/SayRate".to_string();
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
    servers.push(server_rate);

    servers.reverse();
    servers
}

async fn get_clients(server: &VirtualServer) -> HashMap<Path, TungChungClient<Channel>> {
    let mut clients = HashMap::new();
    for (path, addr) in server.conn_addrs().iter() {
        let client = TungChungClient::connect(addr.clone()).await.unwrap();
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

            let mut tung_chung = TungChungImpl::new(local_graphs, clients);
            if server.start_manager() {
                tung_chung.start_manager().await.unwrap();
            }

            log::warn!("Listening on {}...", addr);
            Server::builder()
                .add_service(TungChungServer::new(tung_chung))
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
async fn main() -> Result<(), Box<dyn Error>> {
    init_logging();

    let args = Args::from_args();
    let global_graph = graph::get_global_graph_tung_chung();
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

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use env_logger::{Builder, Env};

use tonic_masa::{Address, GlobalGraph, GraphID, Latency, LocalGraph, Path};

use crate::graph;

#[allow(dead_code)]
pub fn init_logging() {
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

#[allow(dead_code)]
pub fn time_now() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    now as u64
}

#[allow(dead_code)]
pub fn busy_spin(duration: Duration) {
    let now = Instant::now();
    while now.elapsed() < duration {}
}

fn get_global_graph(graph_id: &GraphID) -> GlobalGraph {
    if graph_id == "I2" {
        graph::get_global_graph_i2()
    } else if graph_id == "I2_1" {
        graph::get_global_graph_i2_1()
    } else if graph_id == "I2_2" {
        graph::get_global_graph_i2_2()
    } else {
        panic!("Unsupported graph_id: {}", graph_id);
    }
}

#[allow(dead_code)]
pub fn get_global_graphs(graph_ids: &Vec<GraphID>, slos: &Vec<Latency>) -> Vec<GlobalGraph> {
    let mut global_graphs = Vec::new();
    for i in 0..graph_ids.len() {
        let graph_id = &graph_ids[i];
        let slo = slos[i];
        let mut global_graph = get_global_graph(graph_id);
        assert!(global_graph.graph_id() == graph_id);
        global_graph.set_slo(slo);
        global_graphs.push(global_graph);
    }
    global_graphs
}

#[allow(dead_code)]
pub fn get_local_graphs(
    global_graphs: &Vec<GlobalGraph>,
    path: &Path,
) -> HashMap<Path, LocalGraph> {
    let mut local_graphs = HashMap::new();
    for global_graph in global_graphs.iter() {
        local_graphs.insert(
            global_graph.graph_id().clone(),
            global_graph.get_local_graph(path).clone(),
        );
    }
    local_graphs
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct VirtualServer {
    addr: Address,
    conn_addrs: HashMap<Path, Address>,
    local_graphs: HashMap<Path, LocalGraph>,
    n_threads: usize,
    start_manager: bool,
}

#[allow(dead_code)]
impl VirtualServer {
    pub fn new(
        addr: Address,
        conn_addrs: HashMap<Path, Address>,
        local_graphs: HashMap<Path, LocalGraph>,
        n_threads: usize,
        start_manager: bool,
    ) -> Self {
        let mut paths = Vec::new();
        let mut addrs = Vec::new();
        for (path, addr) in conn_addrs.iter() {
            paths.push(path);
            addrs.push(addr);
        }
        assert_eq!(paths.len(), paths.iter().collect::<HashSet<_>>().len());
        assert_eq!(addrs.len(), addrs.iter().collect::<HashSet<_>>().len());
        Self {
            addr,
            conn_addrs,
            local_graphs,
            n_threads,
            start_manager,
        }
    }

    pub fn addr(&self) -> &Address {
        &self.addr
    }

    pub fn conn_addrs(&self) -> &HashMap<Path, Address> {
        &self.conn_addrs
    }

    pub fn local_graphs(&self) -> &HashMap<Path, LocalGraph> {
        &self.local_graphs
    }

    pub fn n_threads(&self) -> usize {
        self.n_threads
    }

    pub fn start_manager(&self) -> bool {
        self.start_manager
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Span {
    request_id: u64,
    graph_id: String,
    slo: u64,
    latency: u64,
}

#[allow(dead_code)]
impl Span {
    pub fn new(request_id: u64, graph_id: String, slo: u64, latency: u64) -> Self {
        Self {
            request_id,
            graph_id,
            slo,
            latency,
        }
    }
}

#[allow(dead_code)]
pub async fn fetch_traces(output: String, trace_rx: Receiver<Span>) {
    use std::path::Path;
    let path = Path::new(&output);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }
    let mut file = File::create(output).unwrap();
    writeln!(file, "request_id,graph_id,slo,latency").unwrap();
    while let Ok(span) = trace_rx.recv() {
        writeln!(
            file,
            "{},{},{},{}",
            span.request_id, span.graph_id, span.slo, span.latency
        )
        .unwrap();
    }
}

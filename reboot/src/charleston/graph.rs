use std::collections::HashMap;
use tonic::metadata::{Distribution, GlobalGraph, LocalGraph, Path, Span};

pub fn get_global_graph() -> GlobalGraph {
    let local_graphs = {
        let mut graphs = HashMap::new();
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraph::new(vec![Span::new(
                "/hello.Greeter/SayHello".to_string(),
                Distribution::new(1_000),
            )]),
        );
        graphs.insert(
            "/hello.Greeter/SayHello".to_string() as Path,
            LocalGraph::new(vec![
                Span::new("Head".to_string(), Distribution::new(2)),
                Span::new(
                    "/hello.Greeter/SayGoodbye".to_string(),
                    Distribution::new(3_000),
                ),
                Span::new("Tail".to_string(), Distribution::new(4)),
            ]),
        );
        graphs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as Path,
            LocalGraph::new(vec![
                Span::new("Head".to_string(), Distribution::new(5)),
                Span::new("Tail".to_string(), Distribution::new(6)),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new("GID".to_string() as Path, local_graphs);
    global_graph
}

use std::collections::HashMap;
use tonic::metadata::{GlobalGraph, LocalGraph, Path, Span};

pub fn get_global_graph() -> GlobalGraph {
    let local_graphs = {
        let mut graphs = HashMap::new();
        graphs.insert(
            "Source".to_string() as Path,
            LocalGraph::new(vec![Span::new("/hello.Greeter/SayHello".to_string(), 1, 1)]),
        );
        graphs.insert(
            "/hello.Greeter/SayHello".to_string() as Path,
            LocalGraph::new(vec![
                Span::new("Head".to_string(), 2, 2),
                Span::new("/hello.Greeter/SayGoodbye".to_string(), 3, 3),
                Span::new("Tail".to_string(), 4, 4),
            ]),
        );
        graphs.insert(
            "/hello.Greeter/SayGoodbye".to_string() as Path,
            LocalGraph::new(vec![
                Span::new("Head".to_string(), 5, 5),
                Span::new("Tail".to_string(), 6, 6),
            ]),
        );
        graphs
    };
    let global_graph = GlobalGraph::new("GID".to_string() as Path, local_graphs);
    global_graph
}

// src/graph_csv.rs
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use csv::ReaderBuilder;
use serde::Deserialize;

use crate::svc::ServiceName;

/// In-memory representation of a directed call graph (caller -> {callees})
#[derive(Debug, Default)]
pub struct CallGraph {
    /// Outgoing adjacency and weights: caller -> (callee -> weight)
    outgoing: HashMap<ServiceName, HashMap<ServiceName, u64>>,
}

#[derive(Debug, Deserialize)]
struct Row {
    caller: String,
    callee: String,
    #[serde(default)]
    weight: Option<f64>,
}

impl CallGraph {
    /// Build a CallGraph from a CSV reader.
    ///
    /// The CSV must have headers with at least: "caller","callee".
    /// If a "service" column exists, it will be ignored.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self, Box<dyn std::error::Error>> {
        let mut rdr = ReaderBuilder::new()
            .has_headers(true)
            .flexible(true) // allow extra columns
            .trim(csv::Trim::All)
            .from_reader(reader);

        let mut outgoing: HashMap<ServiceName, HashMap<ServiceName, u64>> = HashMap::new();

        for rec in rdr.deserialize::<Row>() {
            let Row {
                caller,
                callee,
                weight,
            } = rec?;
            let caller = caller.trim().to_string();
            let callee = callee.trim().to_string();
            if caller.is_empty() || callee.is_empty() {
                // skip malformed/blank entries
                continue;
            }
            let caller = ServiceName::from_string(caller);
            let callee = ServiceName::from_string(callee);
            let weight = match weight {
                Some(w) => {
                    if !w.is_finite() {
                        return Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "edge weight must be finite",
                        )));
                    }
                    if w < 0.0 {
                        return Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "edge weight must be non-negative",
                        )));
                    }
                    let rounded = w.round();
                    if (w - rounded).abs() > 1e-6 {
                        return Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "edge weight must be an integer value",
                        )));
                    }
                    rounded as u64
                }
                None => 1,
            };

            outgoing.entry(caller).or_default().insert(callee, weight);
        }

        Ok(Self { outgoing })
    }

    /// Build a CallGraph from a CSV file path.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::from_reader(reader)
    }

    /// Return the distinct list of callees that `service` calls (i.e., out-neighbors)
    /// with their associated weights.
    pub fn callees_of(&self, service: &ServiceName) -> HashMap<ServiceName, u64> {
        self.outgoing.get(service).cloned().unwrap_or_default()
    }

    /// Return the weight of the edge caller -> callee, if present.
    pub fn edge_weight(&self, caller: &ServiceName, callee: &ServiceName) -> Option<u64> {
        self.outgoing
            .get(caller)
            .and_then(|targets| targets.get(callee))
            .copied()
    }

    // TODO: create a variant of this that returns an iterator
    pub fn services(&self) -> HashSet<ServiceName> {
        let mut services = HashSet::new();

        for (caller, callees) in &self.outgoing {
            services.insert(caller.clone());
            for callee in callees.keys() {
                services.insert(callee.clone());
            }
        }

        services
    }

    /// Union this call graph with another, merging edges and summing weights.
    pub fn union_with(&mut self, other: &CallGraph) {
        for (caller, callees) in &other.outgoing {
            for (callee, weight) in callees {
                *self.outgoing.entry(caller.clone()).or_default()
                    .entry(callee.clone()).or_insert(0) += weight;
            }
        }
    }

    /// Create a new CallGraph by unioning multiple call graphs.
    pub fn union(mut graphs: Vec<CallGraph>) -> Self {
        if graphs.is_empty() {
            return CallGraph::default();
        }
        
        let mut result = graphs.remove(0);
        for graph in graphs {
            result.union_with(&graph);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_and_queries() {
        let data = r#"service,caller,callee,weight
S1,A,B,3
S1,A,C,5
S2,B,C,2
S3,C,D,1
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");
        let svc_c = ServiceName::new("C");
        let svc_d = ServiceName::new("D");
        let svc_z = ServiceName::new("Z");

        assert_eq!(
            g.callees_of(&svc_a),
            HashMap::from([(svc_b.clone(), 3), (svc_c.clone(), 5),])
        );
        assert_eq!(g.callees_of(&svc_b), HashMap::from([(svc_c.clone(), 2)]));
        assert_eq!(g.callees_of(&svc_c), HashMap::from([(svc_d.clone(), 1)]));
        assert!(g.callees_of(&svc_d).is_empty()); // no outgoing
        assert!(g.callees_of(&svc_z).is_empty()); // unknown service

        assert_eq!(g.edge_weight(&svc_a, &svc_b), Some(3));
        assert_eq!(g.edge_weight(&svc_a, &svc_c), Some(5));
        assert_eq!(g.edge_weight(&svc_b, &svc_c), Some(2));
        assert_eq!(g.edge_weight(&svc_c, &svc_d), Some(1));
        assert_eq!(g.edge_weight(&svc_d, &svc_a), None);
    }

    #[test]
    fn dedup_and_trim() {
        // Duplicates + messy whitespace should dedupe and trim.
        let data = r#"service,caller,callee
S,A,B
S, A , B
S, A ,  B
S, A ,C
S, A , C
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");
        let svc_c = ServiceName::new("C");

        assert_eq!(
            g.callees_of(&svc_a),
            HashMap::from([(svc_b.clone(), 1), (svc_c.clone(), 1),])
        );
    }

    #[test]
    fn ignores_blank_or_malformed_rows() {
        // Rows with empty caller/callee should be skipped.
        let data = r#"service,caller,callee
S1,A,
S1,,B
S1,  ,
S1,A,B
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");

        assert_eq!(g.callees_of(&svc_a), HashMap::from([(svc_b.clone(), 1)]));
    }

    #[test]
    fn missing_weight_defaults_to_one() {
        let data = r#"service,caller,callee
S,A,B
S,A,C
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");
        let svc_c = ServiceName::new("C");

        assert_eq!(g.edge_weight(&svc_a, &svc_b), Some(1));
        assert_eq!(g.edge_weight(&svc_a, &svc_c), Some(1));
    }

    #[test]
    fn non_integer_weights_error() {
        let data = r#"service,caller,callee,weight
S,A,B,1.25
"#;
        let err = CallGraph::from_reader(data.as_bytes()).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("integer") || msg.contains("InvalidData"));
    }

    #[test]
    fn from_path_ok() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "service,caller,callee\nS,A,B\nS,A,C\nS,B,C\n").unwrap();

        let g = CallGraph::from_path(tmp.path()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");
        let svc_c = ServiceName::new("C");

        assert_eq!(
            g.callees_of(&svc_a),
            HashMap::from([(svc_b.clone(), 1), (svc_c.clone(), 1),])
        );
        assert_eq!(g.callees_of(&svc_b), HashMap::from([(svc_c.clone(), 1)]));
    }

    #[test]
    fn extra_columns_are_ignored_with_flexible_csv() {
        // Extra columns beyond service/caller/callee should be ignored thanks to .flexible(true)
        let data = r#"service,caller,callee,extra1,extra2
S,A,B,x,y
S,A,C,foo,bar
S,B,C,zzz,qqq
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");
        let svc_c = ServiceName::new("C");

        assert_eq!(
            g.callees_of(&svc_a),
            HashMap::from([(svc_b.clone(), 1), (svc_c.clone(), 1),])
        );
    }

    #[test]
    fn missing_required_headers_returns_error() {
        // Missing "caller"/"callee" should error during deserialization.
        let data = r#"service,src,dst
S,A,B
"#;
        let err = CallGraph::from_reader(data.as_bytes()).unwrap_err();
        // csv or serde error is fine—just ensure we error out.
        let msg = format!("{err}");
        assert!(
            msg.contains("caller") || msg.contains("deserialize"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn services_list() {
        let data = r#"service,caller,callee
S1,A,B
S1,A,C
S2,B,C
S3,C,D
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();
        let svcs = g.services();

        let expected =
            HashSet::from(["A", "B", "C", "D"].map(|s| ServiceName::from_string(s.to_string())));
        assert_eq!(svcs, expected);
    }
}

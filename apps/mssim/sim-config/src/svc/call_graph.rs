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
    /// Outgoing adjacency: caller -> distinct set of callees
    outgoing: HashMap<ServiceName, HashSet<ServiceName>>,
}

#[derive(Debug, Deserialize)]
struct Row {
    caller: String,
    callee: String,
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

        let mut outgoing: HashMap<ServiceName, HashSet<ServiceName>> = HashMap::new();

        for rec in rdr.deserialize::<Row>() {
            let Row { caller, callee, .. } = rec?;
            let caller = caller.trim().to_string();
            let callee = callee.trim().to_string();
            if caller.is_empty() || callee.is_empty() {
                // skip malformed/blank entries
                continue;
            }
            let caller = ServiceName::from_string(caller);
            let callee = ServiceName::from_string(callee);
            outgoing.entry(caller).or_default().insert(callee);
        }

        Ok(Self { outgoing })
    }

    /// Build a CallGraph from a CSV file path.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::from_reader(reader)
    }

    /// Return the distinct list of callees that `service` calls (i.e., out-neighbors).
    pub fn callees_of(&self, service: &ServiceName) -> HashSet<ServiceName> {
        match self.outgoing.get(service) {
            Some(set) => {
                let v = set.iter().cloned().collect();
                v
            }
            None => HashSet::new(),
        }
    }

    // TODO: create a variant of this that returns an iterator
    pub fn services(&self) -> HashSet<ServiceName> {
        let mut services = HashSet::new();

        for (caller, callees) in &self.outgoing {
            services.insert(caller.clone());
            for callee in callees {
                services.insert(callee.clone());
            }
        }

        services
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_and_queries() {
        let data = r#"service,caller,callee
S1,A,B
S1,A,C
S2,B,C
S3,C,D
"#;
        let g = CallGraph::from_reader(data.as_bytes()).unwrap();

        let svc_a = ServiceName::new("A");
        let svc_b = ServiceName::new("B");
        let svc_c = ServiceName::new("C");
        let svc_d = ServiceName::new("D");
        let svc_z = ServiceName::new("Z");

        assert_eq!(
            g.callees_of(&svc_a),
            HashSet::from([svc_b.clone(), svc_c.clone()])
        );
        assert_eq!(g.callees_of(&svc_b), HashSet::from([svc_c.clone()]));
        assert_eq!(g.callees_of(&svc_c), HashSet::from([svc_d.clone()]));
        assert!(g.callees_of(&svc_d).is_empty()); // no outgoing
        assert!(g.callees_of(&svc_z).is_empty()); // unknown service
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
            HashSet::from([svc_b.clone(), svc_c.clone()])
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

        assert_eq!(g.callees_of(&svc_a), HashSet::from([svc_b.clone()]));
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
            HashSet::from([svc_b.clone(), svc_c.clone()])
        );
        assert_eq!(g.callees_of(&svc_b), HashSet::from([svc_c.clone()]));
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
            HashSet::from([svc_b.clone(), svc_c.clone()])
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

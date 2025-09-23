// src/graph_csv.rs
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use csv::ReaderBuilder;
use serde::Deserialize;

/// In-memory representation of a directed call graph (caller -> {callees})
#[derive(Debug, Default)]
pub struct CallGraph {
    /// Outgoing adjacency: caller -> distinct set of callees
    outgoing: HashMap<String, HashSet<String>>,
}

#[derive(Debug, Deserialize)]
struct Row {
    // Present in the file but ignored for the graph.
    #[serde(default)]
    service: Option<String>,
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

        let mut outgoing: HashMap<String, HashSet<String>> = HashMap::new();

        for rec in rdr.deserialize::<Row>() {
            let Row { caller, callee, .. } = rec?;
            let caller = caller.trim().to_string();
            let callee = callee.trim().to_string();
            if caller.is_empty() || callee.is_empty() {
                // skip malformed/blank entries
                continue;
            }
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
    ///
    /// The result is sorted for stable output. If the service has no outgoing edges
    /// (or does not exist as a caller), an empty Vec is returned.
    pub fn callees_of<S: AsRef<str>>(&self, service: S) -> Vec<String> {
        let s = service.as_ref();
        match self.outgoing.get(s) {
            Some(set) => {
                let mut v: Vec<String> = set.iter().cloned().collect();
                v.sort_unstable();
                v
            }
            None => Vec::new(),
        }
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

        assert_eq!(g.callees_of("A"), vec!["B".to_string(), "C".to_string()]);
        assert_eq!(g.callees_of("B"), vec!["C".to_string()]);
        assert_eq!(g.callees_of("C"), vec!["D".to_string()]);
        assert!(g.callees_of("D").is_empty()); // no outgoing
        assert!(g.callees_of("Z").is_empty()); // unknown service
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
        assert_eq!(g.callees_of("A"), vec!["B".to_string(), "C".to_string()]);
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
        assert_eq!(g.callees_of("A"), vec!["B".to_string()]);
    }

    #[test]
    fn from_path_ok() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "service,caller,callee\nS,A,B\nS,A,C\nS,B,C\n").unwrap();

        let g = CallGraph::from_path(tmp.path()).unwrap();
        assert_eq!(g.callees_of("A"), vec!["B".to_string(), "C".to_string()]);
        assert_eq!(g.callees_of("B"), vec!["C".to_string()]);
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
        assert_eq!(g.callees_of("A"), vec!["B".to_string(), "C".to_string()]);
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
}

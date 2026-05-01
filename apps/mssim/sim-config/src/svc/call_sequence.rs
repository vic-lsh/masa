//! Conditional call sequence parsing for trace-driven service execution.

use crate::svc::{GraphId, MethodId, ServiceName};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::path::PathBuf;

pub type VariantId = String;

/// A downstream call within one fanout step.
#[derive(Clone, Debug, PartialEq)]
pub struct CallSequenceEntry {
    pub service_name: ServiceName,
    pub method_name: MethodId,
    pub callee_variants: Vec<VariantChoice>,
}

/// A possible variant for the downstream call target.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantChoice {
    pub variant_id: VariantId,
    pub probability: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallSequenceVariant {
    pub variant_id: VariantId,
    pub probability: f64,
    pub sequence: Vec<Vec<CallSequenceEntry>>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MethodCallSequence {
    pub variants: HashMap<VariantId, CallSequenceVariant>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CallSequence {
    pub methods: HashMap<MethodId, MethodCallSequence>,
}

/// Error type for call sequence parsing failures.
#[derive(Debug)]
pub struct CallSequenceParseError {
    message: String,
}

impl fmt::Display for CallSequenceParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CallSequenceParseError {}

impl CallSequence {
    pub fn get_method(&self, method_id: &MethodId) -> Option<&MethodCallSequence> {
        self.methods.get(method_id)
    }
}

impl MethodCallSequence {
    pub fn get_variant(&self, variant_id: &str) -> Option<&CallSequenceVariant> {
        self.variants.get(variant_id)
    }
}

impl TryFrom<&str> for CallSequenceEntry {
    type Error = CallSequenceParseError;

    fn try_from(service_method_key: &str) -> Result<Self, Self::Error> {
        let parts: Vec<&str> = service_method_key.split("::").collect();
        if parts.len() != 2 {
            return Err(CallSequenceParseError {
                message: format!(
                    "Invalid call sequence entry format: {}, expected 'service::method'",
                    service_method_key
                ),
            });
        }

        Ok(CallSequenceEntry {
            service_name: ServiceName::from_string(parts[0].to_string()),
            method_name: parts[1].to_string().into(),
            callee_variants: Vec::new(),
        })
    }
}

#[derive(Deserialize)]
struct RawGraph {
    #[allow(dead_code)]
    format: Option<String>,
    methods: HashMap<String, RawMethod>,
}

#[derive(Deserialize)]
struct RawMethod {
    variants: HashMap<String, RawVariant>,
}

#[derive(Deserialize)]
struct RawVariant {
    probability: f64,
    sequence: Vec<Vec<RawCall>>,
}

#[derive(Deserialize)]
struct RawCall {
    target: String,
    #[serde(default)]
    callee_variants: HashMap<String, f64>,
}

/// Loads a conditional call sequence from a new-format `call_sequence.json`.
///
/// The expected schema is:
/// `{ "<graph_id>": { "format": "conditional_variants", "methods": { ... } } }`.
/// Old marginal call sequence files are intentionally unsupported.
pub fn load_call_sequence(config_dir: &PathBuf, graph_id: &GraphId) -> Result<CallSequence> {
    let call_sequence_path = config_dir.join("call_sequence.json");

    let content = std::fs::read_to_string(&call_sequence_path)
        .with_context(|| format!("Failed to read call_sequence.json from {:?}", config_dir))?;

    let graphs: HashMap<String, RawGraph> =
        serde_json::from_str(&content).with_context(|| "Failed to parse call_sequence.json")?;

    let raw_graph = graphs.get(graph_id.as_str()).ok_or_else(|| {
        anyhow::anyhow!(
            "graph_id '{}' not found in call_sequence.json",
            graph_id.as_str()
        )
    })?;

    if raw_graph.format.as_deref() != Some("conditional_variants") {
        anyhow::bail!(
            "call_sequence.json for graph '{}' is not conditional_variants format",
            graph_id.as_str()
        );
    }

    let mut methods = HashMap::new();
    for (method_id, raw_method) in &raw_graph.methods {
        methods.insert(
            normalize_method_key(method_id).into(),
            parse_method_call_sequence(raw_method).with_context(|| {
                format!(
                    "Failed to parse call sequence method '{}' for graph '{}'",
                    method_id,
                    graph_id.as_str()
                )
            })?,
        );
    }

    Ok(CallSequence { methods })
}

/// Get all graph IDs from a new-format `call_sequence.json` file.
pub fn get_all_graph_ids(config_dir: &PathBuf) -> Result<Vec<GraphId>> {
    let call_sequence_path = config_dir.join("call_sequence.json");

    if !call_sequence_path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&call_sequence_path)
        .with_context(|| format!("Failed to read call_sequence.json from {:?}", config_dir))?;

    let graphs: HashMap<String, RawGraph> =
        serde_json::from_str(&content).with_context(|| "Failed to parse call_sequence.json")?;

    Ok(graphs
        .keys()
        .map(|k| GraphId::from_string(k.clone()))
        .collect())
}

fn parse_method_call_sequence(raw_method: &RawMethod) -> Result<MethodCallSequence> {
    let mut variants = HashMap::new();
    for (variant_id, raw_variant) in &raw_method.variants {
        variants.insert(
            variant_id.clone(),
            CallSequenceVariant {
                variant_id: variant_id.clone(),
                probability: raw_variant.probability.clamp(0.0, 1.0),
                sequence: parse_sequence(&raw_variant.sequence)?,
            },
        );
    }
    Ok(MethodCallSequence { variants })
}

fn parse_sequence(raw_sequence: &[Vec<RawCall>]) -> Result<Vec<Vec<CallSequenceEntry>>> {
    raw_sequence
        .iter()
        .map(|raw_step| {
            raw_step
                .iter()
                .map(parse_call_sequence_entry)
                .collect::<Result<Vec<_>>>()
        })
        .collect()
}

fn parse_call_sequence_entry(raw_call: &RawCall) -> Result<CallSequenceEntry> {
    let mut entry = CallSequenceEntry::try_from(raw_call.target.as_str())?;
    entry.callee_variants = parse_variant_choices(&raw_call.callee_variants);
    Ok(entry)
}

fn parse_variant_choices(raw_choices: &HashMap<String, f64>) -> Vec<VariantChoice> {
    let mut choices: Vec<_> = raw_choices
        .iter()
        .map(|(variant_id, probability)| VariantChoice {
            variant_id: variant_id.clone(),
            probability: probability.clamp(0.0, 1.0),
        })
        .collect();
    choices.sort_by(|a, b| a.variant_id.cmp(&b.variant_id));
    choices
}

fn normalize_method_key(method_key: &str) -> String {
    let parts: Vec<&str> = method_key.split("::").collect();
    if parts.len() != 2 {
        return method_key.to_string();
    }

    format!(
        "{}::{}",
        ServiceName::from_string(parts[0].to_string()).as_str(),
        parts[1]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_call_sequence_entry_parsing() {
        let entry = CallSequenceEntry::try_from("service_a::method_b").unwrap();
        assert_eq!(entry.service_name.as_str(), "service-a");
        assert_eq!(entry.method_name, "method_b");
        assert!(entry.callee_variants.is_empty());
    }

    #[test]
    fn test_call_sequence_entry_invalid_format_no_separator() {
        let result = CallSequenceEntry::try_from("invalid");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid call sequence entry format")
        );
    }

    #[test]
    fn test_load_call_sequence_valid_new_format() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();
        fs::write(
            config_dir.join("call_sequence.json"),
            r#"{
  "S_123": {
    "schema_version": 1,
    "format": "conditional_variants",
    "methods": {
      "USER": {
        "count": 10,
        "variants": {
          "v001": {
            "count": 6,
            "probability": 0.6,
            "sequence": [
              [
                {
                  "target": "MS_A::method_1",
                  "callee_variants": { "v003": 1.0 }
                }
              ]
            ]
          },
          "v002": {
            "count": 4,
            "probability": 0.4,
            "sequence": []
          }
        }
      }
    }
  }
}"#,
        )
        .unwrap();

        let graph_id = GraphId::from_string("S_123".to_string());
        let result = load_call_sequence(&config_dir, &graph_id).unwrap();
        let user = result.get_method(&MethodId::from("USER")).unwrap();
        assert_eq!(user.variants.len(), 2);

        let variant = user.get_variant("v001").unwrap();
        assert_eq!(variant.probability, 0.6);
        assert_eq!(variant.sequence.len(), 1);

        let call = &variant.sequence[0][0];
        assert_eq!(call.service_name.as_str(), "ms-a");
        assert_eq!(call.method_name, "method_1");
        assert_eq!(
            call.callee_variants,
            vec![VariantChoice {
                variant_id: "v003".to_string(),
                probability: 1.0
            }]
        );
        assert!(
            result
                .get_method(&MethodId::from("MS_A::method_1"))
                .is_none()
        );
    }

    #[test]
    fn test_load_call_sequence_golden_graph() {
        let config_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/conditional_variants/S_124848862");
        let graph_id = GraphId::from_string("S_124848862".to_string());

        let result = load_call_sequence(&config_dir, &graph_id).unwrap();
        assert_eq!(result.methods.len(), 2);

        let user = result.get_method(&MethodId::from("USER")).unwrap();
        let user_variant = user.get_variant("v001").unwrap();
        assert_eq!(user_variant.probability, 1.0);
        assert_eq!(user_variant.sequence.len(), 1);
        assert_eq!(user_variant.sequence[0].len(), 1);

        let root_call = &user_variant.sequence[0][0];
        assert_eq!(root_call.service_name.as_str(), "ms-10207");
        assert_eq!(root_call.method_name, "0xQx3v-gtz");
        assert_eq!(
            root_call.callee_variants,
            vec![VariantChoice {
                variant_id: "v001".to_string(),
                probability: 1.0
            }]
        );

        let child_method = result
            .get_method(&MethodId::from("ms-10207::0xQx3v-gtz"))
            .unwrap();
        let child_variant = child_method.get_variant("v001").unwrap();
        assert!(child_variant.sequence.is_empty());
    }

    #[test]
    fn test_load_call_sequence_rejects_old_format() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();
        fs::write(
            config_dir.join("call_sequence.json"),
            r#"{
  "S_123": {
    "svc-a": [
      { "svc-b::method": 1.0 }
    ]
  }
}"#,
        )
        .unwrap();

        let graph_id = GraphId::from_string("S_123".to_string());
        let result = load_call_sequence(&config_dir, &graph_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_all_graph_ids() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();
        fs::write(
            config_dir.join("call_sequence.json"),
            r#"{
  "s-123": { "format": "conditional_variants", "methods": {} },
  "S_456": { "format": "conditional_variants", "methods": {} }
}"#,
        )
        .unwrap();

        let mut graph_ids = get_all_graph_ids(&config_dir).unwrap();
        graph_ids.sort();
        assert_eq!(
            graph_ids,
            vec![
                GraphId::from_string("S_123".to_string()),
                GraphId::from_string("S_456".to_string())
            ]
        );
    }
}

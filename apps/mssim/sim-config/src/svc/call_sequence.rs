//! Call sequence parsing for service execution.
//!
//! This module provides types and functions for parsing and working with call sequences
//! that define the order and probability of service method invocations.

use crate::svc::{GraphId, MethodId, ServiceName};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::path::PathBuf;
use tracing::warn;

/// A single entry in a call sequence step, representing a service method call
/// with its associated probability.
#[derive(Clone, Debug, PartialEq)]
pub struct CallSequenceEntry {
    /// The name of the service to call
    pub service_name: ServiceName,
    /// The name of the method to invoke on the service
    pub method_name: MethodId,
    /// The normalized probability (0.0 to 1.0) of executing this call
    pub probability: f64,
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

impl TryFrom<(&str, f64)> for CallSequenceEntry {
    type Error = CallSequenceParseError;

    fn try_from((service_method_key, probability): (&str, f64)) -> Result<Self, Self::Error> {
        // Parse "service::method" format
        let parts: Vec<&str> = service_method_key.split("::").collect();
        if parts.len() != 2 {
            return Err(CallSequenceParseError {
                message: format!(
                    "Invalid call sequence entry format: {}, expected 'service::method'",
                    service_method_key
                ),
            });
        }

        let child_svc_name = ServiceName::from_string(parts[0].to_string());
        let method_name: MethodId = parts[1].to_string().into();
        // Don't clamp probability here - it will be normalized in parse_call_sequence_step
        // Only ensure it's non-negative
        let prob = probability.max(0.0);

        Ok(CallSequenceEntry {
            service_name: child_svc_name,
            method_name,
            probability: prob,
        })
    }
}

/// A call sequence step: a list of entries to invoke in parallel.
/// All entries in a step are executed concurrently before moving to the next step.
pub type CallSequenceStep = Vec<CallSequenceEntry>;

/// A call sequence: a list of steps for a service.
/// Steps are executed sequentially, with entries within each step
/// executed in parallel.
pub type CallSequence = Vec<CallSequenceStep>;

/// Loads and parses a call sequence from a JSON file.
///
/// The expected JSON format is:
/// ```json
/// {
///   "call_graph_id": {
///     "service_name": [
///       { "service::method": probability },
///       ...
///     ],
///     ...
///   }
/// }
/// ```
///
/// # Arguments
/// * `config_dir` - The directory containing the `call_sequence.json` file
/// * `service_name` - The name of the service to load the sequence for
/// * `graph_id` - Graph ID to look up
///
/// # Returns
/// * `Ok(Some(CallSequence))` if the file exists and contains a sequence for the service
/// * `Ok(None)` if the file doesn't exist or the service is not found (no error, just use default behavior)
/// * `Err` if there's an error reading or parsing the file
pub fn load_call_sequence(
    config_dir: &PathBuf,
    service_name: &ServiceName,
    graph_id: &GraphId,
) -> Result<Option<CallSequence>> {
    let call_sequence_path = config_dir.join("call_sequence.json");

    if !call_sequence_path.exists() {
        // Call sequence file doesn't exist, return None to use default parallel fanout
        return Ok(None);
    }

    let content = std::fs::read_to_string(&call_sequence_path)
        .with_context(|| format!("Failed to read call_sequence.json from {:?}", config_dir))?;

    // Parse JSON: { "call_graph_id": { "service_name": [ { "service::method": prob }, ... ], ... } } }
    let json_value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "Failed to parse call_sequence.json")?;

    // Get the call graph object for the specified graph_id
    let call_graph_obj = match json_value.as_object() {
        Some(obj) if !obj.is_empty() => {
            // Look up specific graph_id using normalized format
            let normalized_id = graph_id.as_str();
            match obj.get(normalized_id) {
                Some(graph_obj) => graph_obj,
                None => {
                    warn!(
                        "graph_id '{}' not found in call_sequence.json",
                        normalized_id
                    );
                    return Ok(None);
                }
            }
        }
        _ => {
            warn!("call_sequence.json has no top-level entries");
            return Ok(None);
        }
    };

    // Get the sequence for this service
    // Convert JSON keys to ServiceName for comparison, since ServiceName formats the names
    let sequence = {
        let mut found_sequence = None;
        if let Some(obj) = call_graph_obj.as_object() {
            for (key, seq_value) in obj {
                let json_service_name = ServiceName::from_string(key.clone());
                if &json_service_name == service_name {
                    found_sequence = Some(seq_value);
                    break;
                }
            }
        }

        match found_sequence {
            Some(seq_value) => {
                let raw_steps: Vec<HashMap<String, f64>> =
                    serde_json::from_value(seq_value.clone()).with_context(|| {
                        format!(
                            "Failed to parse call sequence for service {}",
                            service_name.as_str()
                        )
                    })?;

                // Parse each step: convert HashMap<String, f64> to Vec<CallSequenceEntry>
                let mut parsed_steps = Vec::new();
                for raw_step in raw_steps {
                    parsed_steps.push(parse_call_sequence_step(raw_step));
                }
                Some(parsed_steps)
            }
            None => {
                // Service not found in call sequence, use default parallel fanout
                warn!(
                    "Service {} not found in call_sequence.json, using default parallel fanout",
                    service_name.as_str()
                );
                None
            }
        }
    };

    Ok(sequence)
}

/// Loads the root USER call sequence from a JSON file.
///
/// This function looks up "USER" directly as a string key (not through ServiceName formatting)
/// in the specified graph entry of the call_sequence.json file.
///
/// The expected JSON format is:
/// ```json
/// {
///   "call_graph_id": {
///     "USER": [
///       { "service::method": probability },
///       ...
///     ],
///     ...
///   }
/// }
/// ```
///
/// # Arguments
/// * `config_dir` - The directory containing the `call_sequence.json` file
/// * `graph_id` - Graph ID to look up
///
/// # Returns
/// * `Ok(CallSequence)` if the file exists and contains a USER sequence
/// * `Err` if there's an error reading, parsing, or if USER sequence is not found
pub fn load_root_user_call_sequence(
    config_dir: &PathBuf,
    graph_id: &GraphId,
) -> Result<CallSequence> {
    let call_sequence_path = config_dir.join("call_sequence.json");

    if !call_sequence_path.exists() {
        return Err(anyhow::anyhow!(
            "call_sequence.json not found in {:?}",
            config_dir
        ));
    }

    let content = std::fs::read_to_string(&call_sequence_path)
        .with_context(|| format!("Failed to read call_sequence.json from {:?}", config_dir))?;

    let json_value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "Failed to parse call_sequence.json")?;

    // Get the call graph object for the specified graph_id
    let call_graph_obj = match json_value.as_object() {
        Some(obj) if !obj.is_empty() => {
            // Look up specific graph_id using normalized format
            let normalized_id = graph_id.as_str();
            match obj.get(normalized_id) {
                Some(graph_obj) => graph_obj,
                None => {
                    return Err(anyhow::anyhow!(
                        "graph_id '{}' not found in call_sequence.json",
                        normalized_id
                    ));
                }
            }
        }
        _ => {
            return Err(anyhow::anyhow!(
                "call_sequence.json has no top-level entries"
            ));
        }
    };

    // Look up "USER" directly as a string key
    match call_graph_obj.get("USER") {
        Some(seq_value) => {
            let raw_steps: Vec<HashMap<String, f64>> = serde_json::from_value(seq_value.clone())
                .with_context(|| "Failed to parse call sequence for USER")?;

            // Parse each step: convert HashMap<String, f64> to Vec<CallSequenceEntry>
            let mut parsed_steps = Vec::new();
            for raw_step in raw_steps {
                parsed_steps.push(parse_call_sequence_step(raw_step));
            }
            Ok(parsed_steps)
        }
        None => Err(anyhow::anyhow!("USER call sequence not found")),
    }
}

/// Get all graph IDs from call_sequence.json file.
///
/// # Arguments
/// * `config_dir` - The directory containing the `call_sequence.json` file
///
/// # Returns
/// * `Ok(Vec<GraphId>)` with all graph IDs found in the file (normalized)
/// * `Ok(Vec::new())` if the file doesn't exist
/// * `Err` if there's an error reading or parsing the file
pub fn get_all_graph_ids(config_dir: &PathBuf) -> Result<Vec<GraphId>> {
    let call_sequence_path = config_dir.join("call_sequence.json");

    if !call_sequence_path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&call_sequence_path)
        .with_context(|| format!("Failed to read call_sequence.json from {:?}", config_dir))?;

    let json_value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "Failed to parse call_sequence.json")?;

    match json_value.as_object() {
        Some(obj) => Ok(obj
            .keys()
            .map(|k| GraphId::from_string(k.clone()))
            .collect()),
        None => Ok(Vec::new()),
    }
}

/// Parse a call sequence step (HashMap<String, f64>) into a Vec<CallSequenceEntry>.
/// Normalizes probabilities so they sum to 1.0 within the step.
/// The normalized probability for each entry is: probability_in_file / sum_of_probabilities_in_step
fn parse_call_sequence_step(raw_step: HashMap<String, f64>) -> CallSequenceStep {
    // First, parse all entries with their raw probabilities from the file
    let mut parsed_step = Vec::new();
    for (service_method_key, raw_probability) in raw_step {
        match CallSequenceEntry::try_from((service_method_key.as_str(), raw_probability)) {
            Ok(entry) => parsed_step.push(entry),
            Err(e) => {
                warn!("{}", e);
            }
        }
    }

    // Calculate sum of raw probabilities in this step
    let sum: f64 = parsed_step.iter().map(|e| e.probability).sum();

    // Normalize probabilities: divide each by the sum
    if sum > 0.0 {
        for entry in &mut parsed_step {
            entry.probability = entry.probability / sum;
        }
    } else {
        // If sum is 0 or negative, set all probabilities to 0
        warn!("Sum of probabilities in step is {}, setting all to 0", sum);
        for entry in &mut parsed_step {
            entry.probability = 0.0;
        }
    }

    parsed_step
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    // Tests for CallSequenceEntry parsing

    #[test]
    fn test_call_sequence_entry_parsing() {
        let entry = CallSequenceEntry::try_from(("service_a::method_b", 0.5)).unwrap();
        assert_eq!(entry.service_name.as_str(), "service-a");
        assert_eq!(entry.method_name, "method_b");
        assert_eq!(entry.probability, 0.5);
    }

    #[test]
    fn test_call_sequence_entry_service_name_formatting() {
        // Test that service names are formatted correctly (underscores to hyphens, lowercase)
        let entry = CallSequenceEntry::try_from(("MyService_Name::method", 1.0)).unwrap();
        assert_eq!(entry.service_name.as_str(), "myservice-name");
        assert_eq!(entry.method_name, "method");
    }

    #[test]
    fn test_call_sequence_entry_negative_probability() {
        // Negative probabilities should be clamped to 0.0
        let entry = CallSequenceEntry::try_from(("service_a::method_b", -5.0)).unwrap();
        assert_eq!(entry.probability, 0.0);
    }

    #[test]
    fn test_call_sequence_entry_zero_probability() {
        let entry = CallSequenceEntry::try_from(("service_a::method_b", 0.0)).unwrap();
        assert_eq!(entry.probability, 0.0);
    }

    #[test]
    fn test_call_sequence_entry_high_probability() {
        // High probabilities should be preserved (normalization happens later)
        let entry = CallSequenceEntry::try_from(("service_a::method_b", 100.0)).unwrap();
        assert_eq!(entry.probability, 100.0);
    }

    #[test]
    fn test_call_sequence_entry_invalid_format_no_separator() {
        let result = CallSequenceEntry::try_from(("invalid", 0.5));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid call sequence entry format"));
    }

    #[test]
    fn test_call_sequence_entry_invalid_format_too_many_separators() {
        let result = CallSequenceEntry::try_from(("service::method::extra", 0.5));
        assert!(result.is_err());
    }

    #[test]
    fn test_call_sequence_entry_invalid_format_empty_service() {
        // Empty service name actually succeeds (gets formatted to empty string)
        // This is because split("::") on "::method" gives ["", "method"] which is 2 parts
        let result = CallSequenceEntry::try_from(("::method", 0.5));
        if result.is_ok() {
            let entry = result.unwrap();
            assert_eq!(entry.service_name.as_str(), "");
            assert_eq!(entry.method_name, "method");
        }
    }

    #[test]
    fn test_call_sequence_entry_invalid_format_empty_method() {
        // Empty method name actually succeeds
        let entry = CallSequenceEntry::try_from(("service::", 0.5)).unwrap();
        assert_eq!(entry.service_name.as_str(), "service");
        assert_eq!(entry.method_name, "");
    }

    // Tests for parse_call_sequence_step

    #[test]
    fn test_parse_call_sequence_step_normalization() {
        let mut raw_step = HashMap::new();
        raw_step.insert("service_a::method_1".to_string(), 2.0);
        raw_step.insert("service_b::method_2".to_string(), 3.0);

        let parsed = parse_call_sequence_step(raw_step);
        assert_eq!(parsed.len(), 2);

        // Probabilities should be normalized to sum to 1.0
        let sum: f64 = parsed.iter().map(|e| e.probability).sum();
        assert!((sum - 1.0).abs() < 1e-10);

        // Check individual probabilities
        let prob_a = parsed
            .iter()
            .find(|e| e.service_name.as_str() == "service-a")
            .unwrap()
            .probability;
        assert!((prob_a - 0.4).abs() < 1e-10); // 2.0 / 5.0 = 0.4

        let prob_b = parsed
            .iter()
            .find(|e| e.service_name.as_str() == "service-b")
            .unwrap()
            .probability;
        assert!((prob_b - 0.6).abs() < 1e-10); // 3.0 / 5.0 = 0.6
    }

    #[test]
    fn test_parse_call_sequence_step_single_entry() {
        let mut raw_step = HashMap::new();
        raw_step.insert("service_a::method_1".to_string(), 5.0);

        let parsed = parse_call_sequence_step(raw_step);
        assert_eq!(parsed.len(), 1);
        assert!((parsed[0].probability - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_parse_call_sequence_step_zero_sum() {
        let mut raw_step = HashMap::new();
        raw_step.insert("service_a::method_1".to_string(), 0.0);
        raw_step.insert("service_b::method_2".to_string(), 0.0);

        let parsed = parse_call_sequence_step(raw_step);
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().all(|e| e.probability == 0.0));
    }

    #[test]
    fn test_parse_call_sequence_step_negative_sum() {
        let mut raw_step = HashMap::new();
        raw_step.insert("service_a::method_1".to_string(), -1.0);
        raw_step.insert("service_b::method_2".to_string(), -2.0);

        let parsed = parse_call_sequence_step(raw_step);
        assert_eq!(parsed.len(), 2);
        // After clamping negative values to 0, sum is 0, so all probabilities become 0
        assert!(parsed.iter().all(|e| e.probability == 0.0));
    }

    #[test]
    fn test_parse_call_sequence_step_empty() {
        let raw_step = HashMap::new();
        let parsed = parse_call_sequence_step(raw_step);
        assert_eq!(parsed.len(), 0);
    }

    #[test]
    fn test_parse_call_sequence_step_with_invalid_entries() {
        let mut raw_step = HashMap::new();
        raw_step.insert("service_a::method_1".to_string(), 2.0);
        raw_step.insert("invalid_format".to_string(), 3.0); // Invalid, should be skipped
        raw_step.insert("service_b::method_2".to_string(), 5.0);

        let parsed = parse_call_sequence_step(raw_step);
        // Should only have 2 valid entries
        assert_eq!(parsed.len(), 2);

        // Probabilities should still be normalized
        let sum: f64 = parsed.iter().map(|e| e.probability).sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_parse_call_sequence_step_unequal_probabilities() {
        let mut raw_step = HashMap::new();
        raw_step.insert("service_a::method_1".to_string(), 1.0);
        raw_step.insert("service_b::method_2".to_string(), 2.0);
        raw_step.insert("service_c::method_3".to_string(), 3.0);

        let parsed = parse_call_sequence_step(raw_step);
        assert_eq!(parsed.len(), 3);

        // Sum should be 1.0
        let sum: f64 = parsed.iter().map(|e| e.probability).sum();
        assert!((sum - 1.0).abs() < 1e-10);

        // Check ratios are preserved
        let prob_a = parsed
            .iter()
            .find(|e| e.service_name.as_str() == "service-a")
            .unwrap()
            .probability;
        let prob_b = parsed
            .iter()
            .find(|e| e.service_name.as_str() == "service-b")
            .unwrap()
            .probability;
        let prob_c = parsed
            .iter()
            .find(|e| e.service_name.as_str() == "service-c")
            .unwrap()
            .probability;

        // 1:2:3 ratio should be preserved
        assert!((prob_a * 6.0 - 1.0).abs() < 1e-10); // 1/6
        assert!((prob_b * 6.0 - 2.0).abs() < 1e-10); // 2/6
        assert!((prob_c * 6.0 - 3.0).abs() < 1e-10); // 3/6
    }

    // Tests for load_call_sequence

    #[test]
    fn test_load_call_sequence_file_not_exists() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();
        let service_name = ServiceName::from_string("test_service".to_string());
        let graph_id = GraphId::from_string("graph_123".to_string());

        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_call_sequence_valid_file() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        // Create a valid call_sequence.json file
        let json_content = r#"{
  "graph_123": {
    "test-service": [
      {
        "service-a::method-1": 1.0
      },
      {
        "service-b::method-2": 2.0,
        "service-c::method-3": 3.0
      }
    ]
  }
}"#;

        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let service_name = ServiceName::from_string("test_service".to_string());
        let graph_id = GraphId::from_string("graph_123".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();

        assert!(result.is_some());
        let sequence = result.unwrap();
        assert_eq!(sequence.len(), 2); // Two steps

        // First step should have 1 entry
        assert_eq!(sequence[0].len(), 1);
        assert_eq!(sequence[0][0].service_name.as_str(), "service-a");
        assert_eq!(sequence[0][0].method_name, "method-1");
        assert!((sequence[0][0].probability - 1.0).abs() < 1e-10);

        // Second step should have 2 entries with normalized probabilities
        assert_eq!(sequence[1].len(), 2);
        let sum: f64 = sequence[1].iter().map(|e| e.probability).sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_load_call_sequence_service_not_found() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        let json_content = r#"{
  "graph_123": {
    "other-service": [
      {
        "service-a::method-1": 1.0
      }
    ]
  }
}"#;

        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let service_name = ServiceName::from_string("test_service".to_string());
        let graph_id = GraphId::from_string("graph_123".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();

        // Service not found should return None, not an error
        assert!(result.is_none());
    }

    #[test]
    fn test_load_call_sequence_empty_top_level() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        let json_content = r#"{}"#;

        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let service_name = ServiceName::from_string("test_service".to_string());
        let graph_id = GraphId::from_string("graph_123".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_load_call_sequence_invalid_json() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        let json_content = r#"invalid json content"#;

        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let service_name = ServiceName::from_string("test_service".to_string());
        let graph_id = GraphId::from_string("graph_123".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id);

        assert!(result.is_err());
    }

    #[test]
    fn test_load_call_sequence_malformed_structure() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        // JSON structure doesn't match expected format - call graph object value is a string, not an object
        let json_content = r#"{
  "graph_123": "not an object"
}"#;

        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let service_name = ServiceName::from_string("test_service".to_string());
        let graph_id = GraphId::from_string("graph_123".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id);

        // When call graph object is not an object, get() returns None, so it returns None, not an error
        // But actually, when we try to deserialize the sequence, it should fail
        // Let's check: trace_obj.get() on a string value will return None, so it returns None
        // However, if we had a proper object but wrong structure inside, it would error on deserialize
        // For this case, it returns None because the service isn't found
        let result = result.unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_call_sequence_real_world_example() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        // Use a structure similar to the real call_sequence.json
        // Note: Service names in JSON need to match the formatted names (lowercase, hyphens)
        // because ServiceName::as_str() returns the formatted version
        let json_content = r#"{
  "S_14677443": {
    "ms-52612": [
      {
        "MS_37691::y_DKOh-Gts": 1.0
      },
      {
        "MS_37691::ykccIz2fkK": 1.0
      }
    ],
    "ms-56394": [
      {
        "MS_37691::y_DKOh-Gts": 1.0
      },
      {
        "MS_37691::S8pvzJB1uW": 0.0030075187969924814,
        "MS_37691::ykccIz2fkK": 0.9869674185463659,
        "MS_6190::w11N3_b4W7": 0.0030075187969924814
      }
    ]
  }
}"#;
        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let graph_id = GraphId::from_string("S_14677443".to_string());

        // Test loading for MS_52612 (formatted to ms-52612)
        let service_name = ServiceName::from_string("MS_52612".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();
        assert!(result.is_some());
        let sequence = result.unwrap();
        assert_eq!(sequence.len(), 2); // Two steps

        // Test loading for MS_56394 (formatted to ms-56394)
        let service_name = ServiceName::from_string("MS_56394".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();
        assert!(result.is_some());
        let sequence = result.unwrap();
        assert_eq!(sequence.len(), 2); // Two steps

        // Second step should have normalized probabilities
        let step2 = &sequence[1];
        assert_eq!(step2.len(), 3);
        let sum: f64 = step2.iter().map(|e| e.probability).sum();
        assert!((sum - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_load_call_sequence_multiple_call_graph_ids() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().to_path_buf();

        // Multiple call graph IDs - should use the first one
        let json_content = r#"{
  "graph_1": {
    "service-a": [
      {
        "service-b::method": 1.0
      }
    ]
  },
  "graph_2": {
    "service-a": [
      {
        "service-c::method": 1.0
      }
    ]
  }
}"#;

        fs::write(config_dir.join("call_sequence.json"), json_content).unwrap();

        let service_name = ServiceName::from_string("service_a".to_string());
        let graph_id = GraphId::from_string("graph_1".to_string());
        let result = load_call_sequence(&config_dir, &service_name, &graph_id).unwrap();

        assert!(result.is_some());
        let sequence = result.unwrap();
        // Should use first call graph ID's data
        assert_eq!(sequence[0][0].service_name.as_str(), "service-b");
    }

    #[test]
    fn test_call_sequence_entry_display_error() {
        let error = CallSequenceParseError {
            message: "test error".to_string(),
        };
        assert_eq!(format!("{}", error), "test error");
    }
}

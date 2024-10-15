use serde_json;
use std::fs;

pub struct JsonParser {}

impl JsonParser {
    pub fn new() -> JsonParser {
        JsonParser {}
    }

    pub fn read(&self, path: &str) -> serde_json::Value {
        let data = fs::read_to_string(path).expect("Unable to read file");
        let res: serde_json::Value = serde_json::from_str(&data).expect("Unable to parse");
        res
    }
}

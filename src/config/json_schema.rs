//! JSON schema generation for config validation.
use std::sync::OnceLock;

use serde_json::Value;

const CONFIG_SCHEMA_JSON: &str = include_str!("schema/config.schema.json");
static CONFIG_SCHEMA: OnceLock<Value> = OnceLock::new();

pub fn config_schema() -> &'static Value {
    CONFIG_SCHEMA.get_or_init(load_schema)
}

fn load_schema() -> Value {
    serde_json::from_str(CONFIG_SCHEMA_JSON).expect("embedded config schema JSON should be valid")
}

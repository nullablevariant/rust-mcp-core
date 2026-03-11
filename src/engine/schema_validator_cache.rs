//! Engine-owned cache for compiled JSON Schema validators.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use jsonschema::Validator;
use rmcp::ErrorData as McpError;
use serde_json::{Map, Value};

type ValidatorMap = HashMap<Vec<u8>, Arc<Validator>>;

// Shared cache of compiled validators keyed by canonicalized schema bytes.
//
// The cache lives on [`Engine`] so each engine/reload snapshot has isolated
// validator state and no global cross-runtime coupling.
#[derive(Clone, Debug, Default)]
pub(crate) struct SchemaValidatorCache {
    validators: Arc<Mutex<ValidatorMap>>,
}

impl SchemaValidatorCache {
    // Returns a cached compiled validator or compiles and caches it.
    //
    // `invalid_schema` receives the raw compile error text and must map it
    // into the feature-specific MCP error shape.
    pub(crate) fn get_or_compile<F>(
        &self,
        schema: &Value,
        invalid_schema: F,
    ) -> Result<Arc<Validator>, McpError>
    where
        F: Fn(String) -> McpError,
    {
        let cache_key = canonical_schema_bytes(schema).map_err(&invalid_schema)?;

        {
            let guard = self
                .validators
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(cached) = guard.get(&cache_key) {
                return Ok(Arc::clone(cached));
            }
        }

        let compiled = Arc::new(
            jsonschema::validator_for(schema).map_err(|error| invalid_schema(error.to_string()))?,
        );

        let mut guard = self
            .validators
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = guard
            .entry(cache_key)
            .or_insert_with(|| Arc::clone(&compiled));
        Ok(Arc::clone(entry))
    }
}

fn canonical_schema_bytes(schema: &Value) -> Result<Vec<u8>, String> {
    let canonical = canonicalize_json(schema);
    serde_json::to_vec(&canonical).map_err(|error| error.to_string())
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));

            let mut out = Map::with_capacity(entries.len());
            for (key, val) in entries {
                out.insert(key.clone(), canonicalize_json(val));
            }
            Value::Object(out)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
// Inline tests validate private canonicalization helpers used by cache keys.
mod tests {
    use super::canonical_schema_bytes;

    #[test]
    fn canonical_schema_bytes_are_stable_for_key_order() {
        let left = serde_json::json!({
            "type": "object",
            "properties": {
                "b": { "type": "string" },
                "a": { "type": "number" }
            }
        });
        let right = serde_json::json!({
            "properties": {
                "a": { "type": "number" },
                "b": { "type": "string" }
            },
            "type": "object"
        });

        let left_bytes = canonical_schema_bytes(&left).expect("canonical bytes");
        let right_bytes = canonical_schema_bytes(&right).expect("canonical bytes");
        assert_eq!(left_bytes, right_bytes);

        let different = serde_json::json!({
            "type": "object",
            "properties": {
                "a": { "type": "integer" },
                "b": { "type": "string" }
            }
        });
        let different_bytes = canonical_schema_bytes(&different).expect("canonical bytes");
        assert_ne!(left_bytes, different_bytes);
    }
}

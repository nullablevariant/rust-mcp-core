//! JWT claims validation: audience, scope, and custom claim matching.
use std::collections::HashMap;

use serde_json::Value;

pub(super) fn claims_match_required(claims: &Value, required: &HashMap<String, String>) -> bool {
    if required.is_empty() {
        return true;
    }
    for (key, expected) in required {
        let Some(value) = claims.get(key) else {
            return false;
        };
        match value {
            Value::String(actual) if actual == expected => {}
            _ => return false,
        }
    }
    true
}

// Handles both space-delimited string scopes (RFC 6749) and JSON array
// scopes, as different providers use different formats.
pub(super) fn scopes_match_required(claims: &Value, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    let provided = match claims.get("scope") {
        Some(Value::String(scopes)) => scopes
            .split_whitespace()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    required.iter().all(|scope| provided.contains(scope))
}

pub(super) fn audiences_match_required(claims: &Value, required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    let aud = claims.get("aud");
    let provided = match aud {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    required.iter().all(|aud| provided.contains(aud))
}

#[cfg(test)]
// Inline tests here cover private claims/scopes/audiences helpers that are
// not reachable from integration tests without changing visibility.
mod tests {
    use super::{audiences_match_required, claims_match_required, scopes_match_required};
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn claims_match_required_cases() {
        let claims = json!({"azp": "client"});
        let mut required = HashMap::new();
        assert!(claims_match_required(&claims, &required));
        required.insert("azp".to_owned(), "client".to_owned());
        assert!(claims_match_required(&claims, &required));
        required.insert("azp".to_owned(), "other".to_owned());
        assert!(!claims_match_required(&claims, &required));
        required.clear();
        required.insert("missing".to_owned(), "value".to_owned());
        assert!(!claims_match_required(&claims, &required));
        let claims = json!({"azp": 123});
        let mut required = HashMap::new();
        required.insert("azp".to_owned(), "123".to_owned());
        assert!(!claims_match_required(&claims, &required));

        let claims = json!({"azp": "client", "tenant": "acme"});
        let required = HashMap::from([
            ("azp".to_owned(), "client".to_owned()),
            ("tenant".to_owned(), "acme".to_owned()),
        ]);
        assert!(claims_match_required(&claims, &required));

        let missing_key_required = HashMap::from([
            ("azp".to_owned(), "client".to_owned()),
            ("region".to_owned(), "us-east".to_owned()),
        ]);
        assert!(!claims_match_required(&claims, &missing_key_required));

        let mismatched_key_required = HashMap::from([
            ("azp".to_owned(), "client".to_owned()),
            ("tenant".to_owned(), "other".to_owned()),
        ]);
        assert!(!claims_match_required(&claims, &mismatched_key_required));
    }

    #[test]
    fn scopes_match_required_cases() {
        let required = vec!["mcp".to_owned()];
        let claims = json!({"scope": "mcp read"});
        assert!(scopes_match_required(&claims, &required));
        let claims = json!({"scope": ["mcp", "read"]});
        assert!(scopes_match_required(&claims, &required));
        let claims = json!({"scope": ["read"]});
        assert!(!scopes_match_required(&claims, &required));
        let claims = json!({"scope": 123});
        assert!(!scopes_match_required(&claims, &required));
        let empty_required: Vec<String> = Vec::new();
        assert!(scopes_match_required(&claims, &empty_required));

        let required_multi = vec!["mcp".to_owned(), "write".to_owned()];
        let claims_string_full = json!({"scope": "mcp read write"});
        assert!(scopes_match_required(&claims_string_full, &required_multi));
        let claims_string_partial = json!({"scope": "mcp read"});
        assert!(!scopes_match_required(
            &claims_string_partial,
            &required_multi
        ));

        let claims_array_full = json!({"scope": ["read", "mcp", "write"]});
        assert!(scopes_match_required(&claims_array_full, &required_multi));
        let claims_array_partial = json!({"scope": ["mcp"]});
        assert!(!scopes_match_required(
            &claims_array_partial,
            &required_multi
        ));
    }

    #[test]
    fn audiences_match_required_cases() {
        let required = vec!["api://mcp".to_owned()];
        let claims = json!({"aud": "api://mcp"});
        assert!(audiences_match_required(&claims, &required));
        let claims = json!({"aud": ["api://mcp", "other"]});
        assert!(audiences_match_required(&claims, &required));
        let claims = json!({"aud": ["other"]});
        assert!(!audiences_match_required(&claims, &required));
        let claims = json!({"aud": 123});
        assert!(!audiences_match_required(&claims, &required));
        let empty_required: Vec<String> = Vec::new();
        assert!(audiences_match_required(&claims, &empty_required));

        let required_multi = vec!["api://mcp".to_owned(), "api://tasks".to_owned()];
        let claims_string_partial = json!({"aud": "api://mcp"});
        assert!(!audiences_match_required(
            &claims_string_partial,
            &required_multi
        ));

        let claims_array_full = json!({"aud": ["api://mcp", "api://tasks", "other"]});
        assert!(audiences_match_required(
            &claims_array_full,
            &required_multi
        ));
        let claims_array_partial = json!({"aud": ["api://mcp", "other"]});
        assert!(!audiences_match_required(
            &claims_array_partial,
            &required_multi
        ));
    }
}

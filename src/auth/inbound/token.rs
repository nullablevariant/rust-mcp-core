//! JWT token parsing, header decoding, and JWK selection.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use http::HeaderMap;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::Algorithm;
use serde_json::Value;

use crate::config::AuthProviderConfig;

pub(super) fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|auth_header| auth_header.strip_prefix("Bearer "))
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

pub(super) fn token_looks_like_jwt(token: &str) -> bool {
    token.split('.').count() == 3
}

pub(super) fn decode_jwt_claims(token: &str) -> Option<Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&payload).ok()
}

pub(super) fn select_jwk<'a>(jwks: &'a JwkSet, kid: Option<&str>) -> Option<&'a Jwk> {
    if let Some(kid) = kid {
        jwks.keys
            .iter()
            .find(|jwk| jwk.common.key_id.as_deref() == Some(kid))
    } else {
        jwks.keys.first()
    }
}

pub(super) fn parse_algorithms(config: &AuthProviderConfig) -> Vec<Algorithm> {
    if config.algorithms().is_empty() {
        return Vec::new();
    }
    config
        .algorithms()
        .iter()
        .filter_map(|alg| alg.parse().ok())
        .collect()
}

#[cfg(test)]
// Inline tests here cover private token helpers that are not reachable from
// integration tests without changing visibility.
mod tests {
    use super::{decode_jwt_claims, parse_algorithms, select_jwk, token_looks_like_jwt};
    use crate::inline_test_fixtures::base_provider;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use jsonwebtoken::{jwk::JwkSet, Algorithm};
    use serde_json::json;

    #[test]
    fn token_looks_like_jwt_cases() {
        assert!(!token_looks_like_jwt("abc"));
        assert!(!token_looks_like_jwt("a.b"));
        assert!(token_looks_like_jwt("a.b.c"));
        assert!(!token_looks_like_jwt("a.b.c.d"));
        assert!(!token_looks_like_jwt("a.b.c.d.e"));
    }

    #[test]
    fn decode_jwt_claims_invalid_token_returns_none() {
        assert!(decode_jwt_claims("not-a-jwt").is_none());
        assert!(decode_jwt_claims("a.b").is_none());
        assert!(decode_jwt_claims("a.!@#.c").is_none());
        let invalid_json_payload = URL_SAFE_NO_PAD.encode("this-is-not-json");
        let invalid_json_token = format!("a.{invalid_json_payload}.c");
        assert!(decode_jwt_claims(&invalid_json_token).is_none());
    }

    #[test]
    fn decode_jwt_claims_valid_payload() {
        let payload = json!({"iss": "issuer", "aud": "mcp", "sub": "user-123"});
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string());
        let token = format!("a.{payload_b64}.c");
        let claims = decode_jwt_claims(&token).expect("claims should decode");
        assert_eq!(claims, payload);
    }

    #[test]
    fn parse_algorithms_handles_empty_and_valid() {
        let mut provider = base_provider();
        assert!(parse_algorithms(&provider).is_empty());
        provider
            .as_jwks_mut()
            .expect("base_provider must be jwks")
            .algorithms = vec!["HS256".to_owned(), "bad".to_owned()];
        let parsed = parse_algorithms(&provider);
        assert_eq!(parsed, vec![Algorithm::HS256]);
    }

    #[test]
    fn select_jwk_prefers_matching_kid() {
        let jwks_json = r#"{
            "keys": [
                {"kty":"oct","k":"GawgguFyGrWKav7AX4VKUg","kid":"kid1"},
                {"kty":"oct","k":"GawgguFyGrWKav7AX4VKUg","kid":"kid2"}
            ]
        }"#;
        let jwks: JwkSet = serde_json::from_str(jwks_json).expect("jwks parse");
        let selected = select_jwk(&jwks, Some("kid2")).expect("kid2");
        assert_eq!(selected.common.key_id.as_deref(), Some("kid2"));
        let first = select_jwk(&jwks, None).expect("first");
        assert_eq!(first.common.key_id.as_deref(), Some("kid1"));
        assert!(select_jwk(&jwks, Some("missing-kid")).is_none());

        let empty: JwkSet = serde_json::from_str(r#"{"keys": []}"#).expect("empty jwks parse");
        assert!(select_jwk(&empty, None).is_none());
    }
}

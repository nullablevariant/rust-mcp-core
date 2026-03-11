use std::{
    env, fs,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rust_mcp_core::config::{
    SecretValueConfig, SecretValueSource, UpstreamAuth, UpstreamOauth2GrantType,
};
use secrecy::ExposeSecret;
use serde_json::json;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn secret_value_inline_resolves_to_secret_string() {
    let config = SecretValueConfig {
        source: SecretValueSource::Inline,
        value: "inline-secret".to_owned(),
    };

    let secret = config
        .resolve_secret()
        .expect("inline secret should resolve");
    assert_eq!(secret.expose_secret(), "inline-secret");
}

#[test]
fn secret_value_env_resolves_when_present() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    env::set_var("TEST_SECRET_ENV", "env-secret");

    let config = SecretValueConfig {
        source: SecretValueSource::Env,
        value: "TEST_SECRET_ENV".to_owned(),
    };
    let secret = config.resolve_secret().expect("env secret should resolve");
    assert_eq!(secret.expose_secret(), "env-secret");

    env::remove_var("TEST_SECRET_ENV");
}

#[test]
fn secret_value_env_returns_error_when_missing() {
    let _guard = ENV_LOCK.lock().expect("env lock poisoned");
    env::remove_var("TEST_SECRET_ENV_MISSING");

    let config = SecretValueConfig {
        source: SecretValueSource::Env,
        value: "TEST_SECRET_ENV_MISSING".to_owned(),
    };
    let error = config
        .resolve_secret()
        .expect_err("missing env var should return an error");
    assert!(error.contains("TEST_SECRET_ENV_MISSING"));
}

#[test]
fn secret_value_path_reads_and_trims_trailing_newline() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    path.push(format!("tmp-secret-{unique}.txt"));

    fs::write(&path, "file-secret\n").expect("temp secret file should be written");

    let config = SecretValueConfig {
        source: SecretValueSource::Path,
        value: path.to_string_lossy().to_string(),
    };
    let secret = config.resolve_secret().expect("path secret should resolve");
    assert_eq!(secret.expose_secret(), "file-secret");

    let _ = fs::remove_file(path);
}

#[test]
fn secret_value_path_returns_error_when_file_missing() {
    let config = SecretValueConfig {
        source: SecretValueSource::Path,
        value: "/tmp/definitely-missing-secret-file".to_owned(),
    };
    let error = config
        .resolve_secret()
        .expect_err("missing path should return an error");
    assert!(error.contains("definitely-missing-secret-file"));
}

#[test]
fn upstream_auth_deserializes_oauth2_variant() {
    let value = json!({
        "type": "oauth2",
        "grant": "client_credentials",
        "token_url": "https://auth.example.com/oauth/token",
        "client_id": "client",
        "client_secret": {
            "source": "inline",
            "value": "secret"
        }
    });

    let auth: UpstreamAuth = serde_json::from_value(value).expect("oauth2 auth should parse");
    match auth {
        UpstreamAuth::Oauth2(config) => {
            assert_eq!(config.client_id, "client");
            assert_eq!(config.grant, UpstreamOauth2GrantType::ClientCredentials);
        }
        _ => panic!("expected oauth2 auth variant"),
    }
}

#[test]
fn secret_value_debug_redacts_inline_secret_value() {
    let config = SecretValueConfig {
        source: SecretValueSource::Inline,
        value: "super-secret".to_owned(),
    };

    let debug = format!("{config:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("super-secret"));
}

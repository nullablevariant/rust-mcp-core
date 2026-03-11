//! Shared local HTTP mock harness for runnable examples.
//!
//! The harness provides deterministic local endpoints for:
//! - generic upstream HTTP tools (`/items`, `/search`, `/notes`, etc.),
//! - outbound oauth2 token acquisition (`/oauth/token`),
//! - inbound oauth introspection (`/oauth2/introspect`).
//!
//! Examples call `bootstrap_mock_env()` before loading config so endpoint env
//! vars are always available for local, copy/paste validation.

use std::collections::HashMap;
use std::io;

use axum::{
    extract::{Path, Query},
    routing::{get, post},
    Form, Json, Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;

const MOCK_IMAGE_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO5YH0sAAAAASUVORK5CYII=";
const MOCK_AUDIO_BASE64: &str = "UklGRiQAAABXQVZFZm10IBAAAAABAAEAESsAACJWAAACABAAZGF0YQAAAAA=";

#[derive(Debug, Clone)]
struct MockHarness {
    base_url: String,
}

impl MockHarness {
    async fn start() -> Result<Self, io::Error> {
        let app = Router::new()
            .route("/oauth/token", post(oauth_token))
            .route("/oauth2/introspect", post(introspect_token))
            .route("/ping", get(ping))
            .route("/health", get(health))
            .route("/items", get(items_list).post(items_create))
            .route(
                "/items/{id}",
                get(items_read).put(items_update).delete(items_delete),
            )
            .route("/tickets", post(tickets_create))
            .route("/search", get(search))
            .route("/notes", get(notes_list))
            .route("/projects/{id}/summary", get(project_summary))
            .route("/media/{id}", get(media_preview))
            .route("/reports", get(reports_list))
            .route("/billing/{id}", get(billing_get))
            .route("/partners", get(partners_list))
            .route("/one", get(simple_one))
            .route("/two", get(simple_two))
            .route("/alpha", get(simple_alpha))
            .route("/beta", get(simple_beta));

        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let base_url = format!("http://{address}");

        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::warn!("mock harness stopped: {error}");
            }
        });

        Ok(Self { base_url })
    }

    fn apply_default_env(&self) {
        Self::set_if_missing("API_BASE_URL", &self.base_url);
        Self::set_if_missing("STATUS_BASE_URL", &self.base_url);
        Self::set_if_missing("REPORTS_BASE_URL", &self.base_url);
        Self::set_if_missing("BILLING_BASE_URL", &self.base_url);
        Self::set_if_missing("PARTNER_API_BASE_URL", &self.base_url);
        Self::set_if_missing(
            "PARTNER_TOKEN_URL",
            &format!("{}/oauth/token", self.base_url),
        );
        Self::set_if_missing(
            "MCP_INTROSPECTION_URL",
            &format!("{}/oauth2/introspect", self.base_url),
        );
        Self::set_if_missing(
            "MCP_INTROSPECTION_ISSUER",
            &format!("{}/idp", self.base_url),
        );
        Self::set_if_missing("MCP_INTROSPECT_ID", "example-introspect-client");
        Self::set_if_missing("MCP_INTROSPECT_SECRET", "example-introspect-secret");
    }

    fn set_if_missing(key: &str, value: &str) {
        if std::env::var_os(key).is_none() {
            // Example bootstrap intentionally sets deterministic local defaults.
            std::env::set_var(key, value);
        }
    }
}

pub(crate) async fn bootstrap_mock_env() -> Result<(), io::Error> {
    let harness = MockHarness::start().await?;
    harness.apply_default_env();
    tracing::info!("example mock harness running at {}", harness.base_url);
    Ok(())
}

async fn oauth_token(Form(body): Form<HashMap<String, String>>) -> Json<Value> {
    let grant_type = body
        .get("grant_type")
        .map_or("client_credentials", String::as_str);
    let access_token = if grant_type == "refresh_token" {
        "mock-access-token-refresh"
    } else {
        "mock-access-token"
    };
    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "mock-refresh-token",
        "scope": body.get("scope").cloned().unwrap_or_else(|| "mcp.read".to_owned()),
    }))
}

async fn introspect_token(Form(body): Form<HashMap<String, String>>) -> Json<Value> {
    let token = body.get("token").map(String::as_str).unwrap_or_default();
    let active = token == "test-oauth-access-token"
        || token == "mock-access-token"
        || token == "mock-access-token-refresh";
    let issuer = std::env::var("MCP_INTROSPECTION_ISSUER")
        .ok()
        .unwrap_or_else(|| "http://127.0.0.1/idp".to_owned());
    Json(json!({
        "active": active,
        "scope": "mcp.read",
        "aud": ["mcp"],
        "iss": issuer,
        "sub": "example-user",
        "exp": 4_102_444_800_u64,
    }))
}

async fn ping() -> Json<Value> {
    Json(json!({"pong": true}))
}

async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn items_list(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
    Json(json!({
        "items": [
            {"id": "item-1", "name": "alpha"},
            {"id": "item-2", "name": "beta"}
        ],
        "query": query
    }))
}

async fn items_read(Path(id): Path<String>) -> Json<Value> {
    Json(json!({"id": id, "name": format!("item-{id}")}))
}

async fn items_create(Json(body): Json<Value>) -> Json<Value> {
    Json(json!({"created": true, "id": "item-new", "received": body}))
}

async fn items_update(Path(id): Path<String>, Json(body): Json<Value>) -> Json<Value> {
    Json(json!({"updated": true, "id": id, "received": body}))
}

async fn items_delete(Path(id): Path<String>) -> Json<Value> {
    Json(json!({"deleted": true, "id": id}))
}

async fn tickets_create(Json(body): Json<Value>) -> Json<Value> {
    Json(json!({"id": "ticket-1", "status": "created", "received": body}))
}

async fn search(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
    let term = query.get("q").cloned();
    Json(json!({
        "query": term,
        "query_params": query,
        "results": [
            {"title": "Rust MCP docs", "url": "https://example.local/docs"},
            {"title": "MCP spec", "url": "https://example.local/spec"}
        ]
    }))
}

async fn notes_list(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
    let limit = query
        .get("limit")
        .cloned()
        .unwrap_or_else(|| "10".to_owned());
    Json(json!({
        "items": [
            {"id": "note-1", "title": "First", "limit": limit}
        ]
    }))
}

async fn project_summary(Path(id): Path<String>) -> Json<Value> {
    Json(json!({"summary": format!("Summary for project {id}")}))
}

async fn media_preview(Path(id): Path<String>) -> Json<Value> {
    Json(json!({
        "id": id,
        "image_base64": MOCK_IMAGE_BASE64,
        "audio_base64": MOCK_AUDIO_BASE64,
        "summary": "Mock media summary"
    }))
}

async fn reports_list() -> Json<Value> {
    Json(json!({
        "reports": [
            {"id": "report-1", "status": "ready"},
            {"id": "report-2", "status": "processing"}
        ]
    }))
}

async fn billing_get(Path(id): Path<String>) -> Json<Value> {
    Json(json!({"id": id, "status": "paid"}))
}

async fn partners_list(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
    Json(json!({
        "partners": [
            {"id": "partner-1", "name": "Contoso"},
            {"id": "partner-2", "name": "Fabrikam"}
        ],
        "query": query
    }))
}

async fn simple_one() -> Json<Value> {
    Json(json!({"value": "one"}))
}

async fn simple_two() -> Json<Value> {
    Json(json!({"value": "two"}))
}

async fn simple_alpha() -> Json<Value> {
    Json(json!({"value": "alpha"}))
}

async fn simple_beta() -> Json<Value> {
    Json(json!({"value": "beta"}))
}

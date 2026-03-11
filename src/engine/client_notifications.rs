//! Client notification hub for broadcasting list-changed and task status events.
use std::fmt;
use std::sync::Arc;

use rmcp::service::{Peer, RequestContext, RoleServer};
use tokio::sync::Mutex;
use tracing::warn;

#[derive(Clone, Default)]
pub struct ClientNotificationHub {
    peers: Arc<Mutex<Vec<ObservedPeer>>>,
}

impl fmt::Debug for ClientNotificationHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientNotificationHub")
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct ObservedPeer {
    key: String,
    peer: Peer<RoleServer>,
}

impl ClientNotificationHub {
    pub(crate) async fn observe_peer(&self, context: &RequestContext<RoleServer>) {
        let key = peer_key(context);
        let peer = context.peer.clone();
        let mut peers = self.peers.lock().await;
        peers.retain(|entry| !entry.peer.is_transport_closed());
        if let Some(existing) = peers.iter_mut().find(|entry| entry.key == key) {
            existing.peer = peer;
            return;
        }
        peers.push(ObservedPeer { key, peer });
    }

    pub(crate) async fn notify_tools_list_changed(&self) -> usize {
        self.notify(NotificationKind::Tools).await
    }

    pub(crate) async fn notify_prompts_list_changed(&self) -> usize {
        self.notify(NotificationKind::Prompts).await
    }

    pub(crate) async fn notify_resources_list_changed(&self) -> usize {
        self.notify(NotificationKind::Resources).await
    }

    // Drains the peer list, sends notifications, and re-merges successful
    // peers back. Draining prevents holding the lock during async sends.
    // Failed peers are dropped (likely disconnected).
    async fn notify(&self, kind: NotificationKind) -> usize {
        let pending = {
            let mut peers = self.peers.lock().await;
            peers.retain(|entry| !entry.peer.is_transport_closed());
            if peers.is_empty() {
                return 0;
            }
            std::mem::take(&mut *peers)
        };

        if pending.is_empty() {
            return 0;
        }

        let mut sent = 0usize;
        let mut keep = Vec::with_capacity(pending.len());
        for entry in pending {
            let result = match kind {
                NotificationKind::Tools => entry.peer.notify_tool_list_changed().await,
                NotificationKind::Prompts => entry.peer.notify_prompt_list_changed().await,
                NotificationKind::Resources => entry.peer.notify_resource_list_changed().await,
            };

            match result {
                Ok(()) => {
                    sent += 1;
                    keep.push(entry);
                }
                Err(error) => {
                    warn!(
                        "failed to send {} list_changed notification: {}",
                        kind.label(),
                        error
                    );
                }
            }
        }

        let mut peers = self.peers.lock().await;
        peers.retain(|entry| !entry.peer.is_transport_closed());
        merge_peers(&mut peers, keep);
        sent
    }
}

fn merge_peers(peers: &mut Vec<ObservedPeer>, candidates: Vec<ObservedPeer>) {
    for candidate in candidates {
        if peers.iter().any(|entry| entry.key == candidate.key) {
            continue;
        }
        peers.push(candidate);
    }
}

// Derives a stable key to deduplicate peers: prefers the MCP-Session-Id
// header (streamable HTTP), falls back to peer_info pointer identity (stdio).
pub(crate) fn peer_key(context: &RequestContext<RoleServer>) -> String {
    if let Some(parts) = context.extensions.get::<http::request::Parts>() {
        if let Some(session_id) = parts
            .headers
            .get("MCP-Session-Id")
            .and_then(|value| value.to_str().ok())
        {
            return format!("session:{session_id}");
        }
    }
    if let Some(peer_info) = context.peer.peer_info() {
        return format!("peer_info:{peer_info:p}");
    }
    "peer:uninitialized".to_owned()
}

enum NotificationKind {
    Tools,
    Prompts,
    Resources,
}

impl NotificationKind {
    const fn label(&self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Prompts => "prompts",
            Self::Resources => "resources",
        }
    }
}

#[cfg(test)]
// Inline tests are used because this module is crate-private and depends on
// private runtime wiring details that are not reachable from external tests.
mod tests {
    use super::ClientNotificationHub;
    use crate::engine::Engine;
    use crate::inline_test_fixtures::{base_config, read_frame};
    use rmcp::{
        model::{Extensions, Meta, NumberOrString},
        service::RequestContext,
    };
    use serde::Deserialize;
    use tokio::io::AsyncReadExt;
    use tokio_util::sync::CancellationToken;

    fn request_context(
        service: &rmcp::service::RunningService<rmcp::service::RoleServer, Engine>,
    ) -> RequestContext<rmcp::service::RoleServer> {
        let context = RequestContext {
            peer: service.peer().clone(),
            ct: CancellationToken::new(),
            id: NumberOrString::Number(1),
            meta: Meta::default(),
            extensions: Extensions::default(),
        };
        if context.peer.peer_info().is_none() {
            let client_info = rmcp::model::Implementation::new("test-client", "1.0.0");
            context
                .peer
                .set_peer_info(rmcp::model::InitializeRequestParams::new(
                    rmcp::model::ClientCapabilities::default(),
                    client_info,
                ));
        }
        context
    }

    // Extracts the first JSON object from a raw frame string.
    // rmcp frames may contain a length-prefixed header before the JSON body
    // and trailing data after the object.
    fn parse_jsonrpc_notification(frame: &str) -> serde_json::Value {
        let json_start = frame.find('{').expect("frame must contain JSON object");
        let json_str = &frame[json_start..];
        let mut deserializer = serde_json::Deserializer::from_str(json_str);
        serde_json::Value::deserialize(&mut deserializer).expect("frame must contain valid JSON")
    }

    // Asserts that a JSON-RPC notification frame has the expected method and
    // conforms to the list-changed notification structure (jsonrpc 2.0, no id/params).
    fn assert_jsonrpc_notification(frame: &str, expected_method: &str) {
        let parsed = parse_jsonrpc_notification(frame);
        assert_eq!(
            parsed.get("jsonrpc").and_then(|v| v.as_str()),
            Some("2.0"),
            "frame must be JSON-RPC 2.0"
        );
        assert_eq!(
            parsed.get("method").and_then(|v| v.as_str()),
            Some(expected_method),
            "notification method must match"
        );
        assert!(
            parsed.get("id").is_none(),
            "notifications must not have an id field"
        );
        assert!(
            parsed.get("params").is_none(),
            "list_changed notifications must not include params"
        );
    }

    async fn peer_count(hub: &ClientNotificationHub) -> usize {
        hub.peers.lock().await.len()
    }

    #[tokio::test]
    async fn notify_returns_zero_when_no_peers_registered() {
        let hub = ClientNotificationHub::default();
        assert_eq!(hub.notify_tools_list_changed().await, 0);
        assert_eq!(hub.notify_prompts_list_changed().await, 0);
        assert_eq!(hub.notify_resources_list_changed().await, 0);
    }

    // Remediation 1 + 2: parse JSON-RPC payload for exact method+params contract;
    // assert idempotent registration semantics with repeated observe_peer calls.
    #[tokio::test]
    async fn observe_peer_tracks_each_seen_peer() {
        let hub = ClientNotificationHub::default();
        let engine = Engine::new(base_config()).expect("engine");
        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly(engine, server_io, None);

        let context = request_context(&service);

        // First registration adds the peer.
        hub.observe_peer(&context).await;
        assert_eq!(peer_count(&hub).await, 1, "first observe must add one peer");

        // Repeated registration is idempotent — peer count must not grow.
        hub.observe_peer(&context).await;
        assert_eq!(
            peer_count(&hub).await,
            1,
            "duplicate observe must not add another peer"
        );

        // A third call still keeps exactly one peer.
        hub.observe_peer(&context).await;
        assert_eq!(
            peer_count(&hub).await,
            1,
            "triple observe must remain idempotent"
        );

        let sent = hub.notify_tools_list_changed().await;
        assert_eq!(sent, 1);

        // Parse and validate the JSON-RPC notification payload.
        let frame = read_frame(&mut client_io).await.expect("tools frame");
        assert_jsonrpc_notification(&frame, "notifications/tools/list_changed");

        let _ = service.close().await;
    }

    // Remediation 1: parse emitted JSON-RPC notification payloads for prompts
    // and resources, asserting exact method contract.
    #[tokio::test]
    async fn notify_prompts_and_resources_frames_are_emitted() {
        let hub = ClientNotificationHub::default();
        let engine = Engine::new(base_config()).expect("engine");
        let (server_io, mut client_io) = tokio::io::duplex(4096);
        let mut service = rmcp::service::serve_directly(engine, server_io, None);

        let context = request_context(&service);
        hub.observe_peer(&context).await;

        let prompt_sent = hub.notify_prompts_list_changed().await;
        assert_eq!(prompt_sent, 1);
        let prompt_frame = read_frame(&mut client_io).await.expect("prompt frame");
        assert_jsonrpc_notification(&prompt_frame, "notifications/prompts/list_changed");

        let resource_sent = hub.notify_resources_list_changed().await;
        assert_eq!(resource_sent, 1);
        let resource_frame = read_frame(&mut client_io).await.expect("resource frame");
        assert_jsonrpc_notification(&resource_frame, "notifications/resources/list_changed");

        let _ = service.close().await;
    }

    // Remediation 3: assert closed peers are pruned across multiple notify
    // call types and remain pruned on subsequent calls.
    #[tokio::test]
    async fn closed_peers_are_dropped_after_failed_send() {
        let hub = ClientNotificationHub::default();
        let engine = Engine::new(base_config()).expect("engine");
        let (server_io, _client_io) = tokio::io::duplex(1024);
        let mut service = rmcp::service::serve_directly(engine, server_io, None);

        let context = request_context(&service);
        hub.observe_peer(&context).await;
        assert_eq!(
            peer_count(&hub).await,
            1,
            "peer must be registered before close"
        );

        let _ = service.close().await;

        // First notify detects the closed peer and prunes it.
        assert_eq!(hub.notify_tools_list_changed().await, 0);
        assert_eq!(
            peer_count(&hub).await,
            0,
            "closed peer must be pruned after failed tools notify"
        );

        // Second notify still returns zero — peer stays pruned.
        assert_eq!(hub.notify_tools_list_changed().await, 0);
        assert_eq!(
            peer_count(&hub).await,
            0,
            "peer list must remain empty on subsequent tools notify"
        );

        // Prompts and resources also see zero — no stale peer reappears.
        assert_eq!(hub.notify_prompts_list_changed().await, 0);
        assert_eq!(
            peer_count(&hub).await,
            0,
            "peer list must remain empty after prompts notify"
        );
        assert_eq!(hub.notify_resources_list_changed().await, 0);
        assert_eq!(
            peer_count(&hub).await,
            0,
            "peer list must remain empty after resources notify"
        );
    }

    // Remediation 3: closed peers are pruned for prompt and resource notification types.
    #[tokio::test]
    async fn closed_peers_fail_for_prompt_and_resource_notifications() {
        let hub = ClientNotificationHub::default();
        let engine = Engine::new(base_config()).expect("engine");
        let (server_io, _client_io) = tokio::io::duplex(1024);
        let mut service = rmcp::service::serve_directly(engine, server_io, None);

        let context = request_context(&service);
        hub.observe_peer(&context).await;
        assert_eq!(
            peer_count(&hub).await,
            1,
            "peer must be registered before close"
        );

        let _ = service.close().await;

        assert_eq!(hub.notify_prompts_list_changed().await, 0);
        assert_eq!(
            peer_count(&hub).await,
            0,
            "closed peer must be pruned after failed prompts notify"
        );

        // Resources notify on already-empty list stays at zero.
        assert_eq!(hub.notify_resources_list_changed().await, 0);
        assert_eq!(
            peer_count(&hub).await,
            0,
            "peer list must remain empty after resources notify"
        );
    }

    // Remediation 4: add bounded-time assertion and explicit "no bytes read" invariant.
    #[tokio::test]
    async fn read_frame_times_out_when_no_notification_is_sent() {
        let (_server_io, mut client_io) = tokio::io::duplex(128);

        let start = std::time::Instant::now();
        let result = read_frame(&mut client_io).await;
        let elapsed = start.elapsed();

        assert!(result.is_none(), "must return None when no data is sent");

        // read_frame uses a 250ms timeout; assert we completed in bounded time
        // (allowing some scheduling slack but proving the timeout path fired).
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "read_frame must complete within bounded time, took {elapsed:?}"
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(200),
            "read_frame should wait near the 250ms timeout, completed too fast at {elapsed:?}"
        );

        // Explicit no-bytes-read invariant: after timeout, a direct read still
        // has no data available and must time out quickly.
        let mut buf = [0_u8; 1];
        let direct_read = tokio::time::timeout(
            std::time::Duration::from_millis(25),
            client_io.read(&mut buf),
        )
        .await;
        assert!(
            direct_read.is_err(),
            "no bytes should be readable after timeout path"
        );
    }
}

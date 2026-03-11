//! Shared OAuth abstractions used by inbound and outbound auth flows.

pub mod clock;
#[cfg(any(feature = "auth", feature = "http_tools"))]
pub mod http_bridge;
pub mod token_exchange;

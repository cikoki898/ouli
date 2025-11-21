//! Network layer for Ouli
//!
//! Provides async HTTP and WebSocket handling

pub mod client;
pub mod connection_pool;
pub mod handler;
pub mod http;
pub mod websocket;

pub use client::{ForwardRequest, ForwardedResponse, HttpClient};
pub use connection_pool::{ConnectionGuard, ConnectionPool};
pub use handler::NetworkHandler;
pub use http::HttpHandler;
pub use websocket::WebSocketHandler;

/// Maximum number of concurrent connections
pub const MAX_CONNECTIONS: usize = 4096;

/// Connection setup timeout
pub const CONNECT_TIMEOUT_MS: u64 = 1000;

/// Graceful shutdown timeout
pub const SHUTDOWN_TIMEOUT_MS: u64 = 5000;

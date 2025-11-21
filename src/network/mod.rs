//! Network layer for Ouli
//!
//! Provides async HTTP/WebSocket handling with bounded concurrency.

mod connection_pool;
mod handler;
mod http;
mod websocket;

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

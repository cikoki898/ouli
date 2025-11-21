//! Proxy integration for recording and replay

mod http;
mod websocket;

pub use http::HttpProxy;
pub use websocket::WebSocketProxy;

use crate::config::Mode;

/// Proxy mode determines behavior
impl Mode {
    /// Check if mode is Record
    #[must_use]
    pub fn is_record(&self) -> bool {
        matches!(self, Mode::Record)
    }

    /// Check if mode is Replay
    #[must_use]
    pub fn is_replay(&self) -> bool {
        matches!(self, Mode::Replay)
    }
}

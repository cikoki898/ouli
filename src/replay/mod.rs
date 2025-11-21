//! Replay engine for serving recorded HTTP/WebSocket traffic

mod cache;
mod engine;

pub use cache::ReplayCache;
pub use engine::ReplayEngine;

/// Cache warming strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmingStrategy {
    /// Load all recordings on startup
    Eager,
    /// Load recordings on first access
    Lazy,
}

impl Default for WarmingStrategy {
    fn default() -> Self {
        Self::Lazy
    }
}

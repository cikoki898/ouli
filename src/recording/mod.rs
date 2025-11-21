//! Recording engine for capturing HTTP/WebSocket traffic

mod engine;
mod session;

pub use engine::{RecordingEngine, Response};
pub use session::{RecordingSession, SessionManager};

/// Maximum number of concurrent recording sessions
pub const MAX_SESSIONS: usize = 1024;

/// Default recording session if no test name provided
pub const DEFAULT_SESSION: &str = "default";

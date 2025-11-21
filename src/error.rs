//! Error types for Ouli

use std::io;
use thiserror::Error;

/// Result type for Ouli operations
pub type Result<T> = std::result::Result<T, OuliError>;

/// Errors that can occur in Ouli
#[derive(Debug, Error)]
pub enum OuliError {
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Invalid recording file format
    #[error("Invalid recording format: {0}")]
    InvalidFormat(String),

    /// Recording file corrupted (CRC mismatch)
    #[error("Recording corrupted at offset {offset}: expected CRC {expected:#x}, got {actual:#x}")]
    CorruptedData {
        /// Offset where corruption was detected
        offset: u64,
        /// Expected CRC32 value
        expected: u32,
        /// Actual CRC32 value
        actual: u32,
    },

    /// Request hash not found in recording
    #[error("Recording not found for hash {0:x?}")]
    RecordingNotFound([u8; 32]),

    /// Recording file not found
    #[error("Recording file not found: {0}")]
    FileNotFound(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Request/response too large
    #[error("Data too large: {size} bytes exceeds limit of {limit} bytes")]
    DataTooLarge {
        /// Actual size
        size: usize,
        /// Size limit
        limit: usize,
    },

    /// Invalid test name
    #[error("Invalid test name: {0}")]
    InvalidTestName(String),

    /// Generic error with context
    #[error("{0}")]
    Other(String),
}

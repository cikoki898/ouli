//! Ouli - High-performance HTTP/WebSocket record-replay proxy
//!
//! Built with `TigerBeetle` principles for deterministic testing.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs, clippy::all, clippy::pedantic, clippy::cargo)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::field_reassign_with_default,
    clippy::multiple_crate_versions
)]

pub mod config;
pub mod error;
pub mod fingerprint;
pub mod network;
pub mod proxy;
pub mod recording;
pub mod replay;
pub mod storage;

pub use error::{OuliError, Result};

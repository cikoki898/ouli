//! Binary storage format for recordings

mod format;
mod reader;
mod writer;

pub use format::{
    FileHeader, InteractionEntry, RequestHeader, ResponseHeader, CHAIN_DEPTH_MAX, FILE_MAGIC,
    FILE_VERSION, HEADER_SIZE, INDEX_ENTRY_SIZE,
};
pub use reader::RecordingReader;
pub use writer::RecordingWriter;

use crate::Result;

/// Validate recording file magic and version
///
/// # Errors
///
/// Returns error if magic or version is invalid
pub fn validate_header(header: &FileHeader) -> Result<()> {
    if header.magic != FILE_MAGIC {
        return Err(crate::OuliError::InvalidFormat(format!(
            "Invalid magic bytes: expected {:?}, got {:?}",
            FILE_MAGIC, header.magic
        )));
    }

    if header.version != FILE_VERSION {
        return Err(crate::OuliError::InvalidFormat(format!(
            "Unsupported version: {}, expected {}",
            header.version, FILE_VERSION
        )));
    }

    Ok(())
}

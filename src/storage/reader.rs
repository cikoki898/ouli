//! Recording file reader

use std::fs::File;
use std::path::Path;

use bytemuck::from_bytes;
use crc32fast::Hasher;
use memmap2::Mmap;

use super::format::{FileHeader, InteractionEntry, INDEX_ENTRY_SIZE};
use crate::{OuliError, Result};

/// Reader for recording files
pub struct RecordingReader {
    _file: File,
    mmap: Mmap,
    header: FileHeader,
}

impl RecordingReader {
    /// Open an existing recording file
    ///
    /// # Errors
    ///
    /// Returns error if file cannot be opened, mapped, or is invalid
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        // Read and validate header
        if mmap.len() < super::HEADER_SIZE {
            return Err(OuliError::InvalidFormat(
                "File too small to contain header".to_string(),
            ));
        }

        let header: FileHeader = *from_bytes(&mmap[..super::HEADER_SIZE]);

        // Validate magic and version
        super::validate_header(&header)?;

        // Verify header CRC (exclude CRC field at bytes 12-15)
        let mut hasher = Hasher::new();
        hasher.update(&mmap[..12]); // magic + version
        hasher.update(&mmap[16..super::HEADER_SIZE]); // rest of header after CRC
        let computed_crc = hasher.finalize();

        if header.header_crc != computed_crc {
            return Err(OuliError::CorruptedData {
                offset: 0,
                expected: header.header_crc,
                actual: computed_crc,
            });
        }

        Ok(Self {
            _file: file,
            mmap,
            header,
        })
    }

    /// Get the number of interactions in this recording
    #[must_use]
    pub fn interaction_count(&self) -> u64 {
        self.header.interaction_count
    }

    /// Get the recording ID
    #[must_use]
    pub fn recording_id(&self) -> [u8; 32] {
        self.header.recording_id
    }

    /// Get the final chain state
    #[must_use]
    pub fn final_chain_state(&self) -> [u8; 32] {
        self.header.final_chain_state
    }

    /// Lookup an interaction by request hash
    #[must_use]
    pub fn lookup(&self, request_hash: [u8; 32]) -> Option<InteractionEntry> {
        // Linear search through index
        // TODO: Binary search or hash table for O(1) lookup
        let index_start = super::HEADER_SIZE;
        let count = self.header.interaction_count as usize;

        for i in 0..count {
            let offset = index_start + (i * INDEX_ENTRY_SIZE);
            let entry: InteractionEntry =
                *from_bytes(&self.mmap[offset..offset + INDEX_ENTRY_SIZE]);

            if entry.request_hash == request_hash {
                return Some(entry);
            }
        }

        None
    }

    /// Get all index entries
    #[must_use]
    pub fn all_entries(&self) -> Vec<InteractionEntry> {
        let mut entries = Vec::with_capacity(self.header.interaction_count as usize);
        let index_start = super::HEADER_SIZE;

        for i in 0..self.header.interaction_count as usize {
            let offset = index_start + (i * INDEX_ENTRY_SIZE);
            let entry: InteractionEntry =
                *from_bytes(&self.mmap[offset..offset + INDEX_ENTRY_SIZE]);
            entries.push(entry);
        }

        entries
    }

    /// Read request data for an interaction
    ///
    /// # Errors
    ///
    /// Returns error if offset is invalid
    pub fn read_request(&self, entry: &InteractionEntry) -> Result<&[u8]> {
        let start = entry.request_offset as usize;
        let end = start + entry.request_size as usize;

        if end > self.mmap.len() {
            return Err(OuliError::InvalidFormat(format!(
                "Request data extends beyond file: {end} > {}",
                self.mmap.len()
            )));
        }

        Ok(&self.mmap[start..end])
    }

    /// Read response data for an interaction
    ///
    /// # Errors
    ///
    /// Returns error if offset is invalid
    pub fn read_response(&self, entry: &InteractionEntry) -> Result<&[u8]> {
        let start = entry.response_offset as usize;
        let end = start + entry.response_size as usize;

        if end > self.mmap.len() {
            return Err(OuliError::InvalidFormat(format!(
                "Response data extends beyond file: {end} > {}",
                self.mmap.len()
            )));
        }

        Ok(&self.mmap[start..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::RecordingWriter;
    use tempfile::NamedTempFile;

    #[test]
    fn test_roundtrip() {
        let file = NamedTempFile::new().unwrap();
        let recording_id = [42u8; 32];

        // Write
        {
            let mut writer = RecordingWriter::create(file.path(), recording_id).unwrap();

            let request_hash = [1u8; 32];
            let prev_hash = [0u8; 32];
            let request_data = b"GET /test HTTP/1.1\r\n\r\n";
            let response_data = b"HTTP/1.1 200 OK\r\n\r\nHello";

            writer
                .append_interaction(request_hash, prev_hash, request_data, response_data)
                .unwrap();

            writer
                .finalize(crate::fingerprint::CHAIN_HEAD_HASH)
                .unwrap();
        }

        // Read
        {
            let reader = RecordingReader::open(file.path()).unwrap();

            assert_eq!(reader.recording_id(), recording_id);
            assert_eq!(reader.interaction_count(), 1);

            let request_hash = [1u8; 32];
            let entry = reader.lookup(request_hash).unwrap();

            let request = reader.read_request(&entry).unwrap();
            assert_eq!(request, b"GET /test HTTP/1.1\r\n\r\n");

            let response = reader.read_response(&entry).unwrap();
            assert_eq!(response, b"HTTP/1.1 200 OK\r\n\r\nHello");
        }
    }

    #[test]
    fn test_multiple_interactions() {
        let file = NamedTempFile::new().unwrap();
        let recording_id = [99u8; 32];

        // Write multiple
        {
            let mut writer = RecordingWriter::create(file.path(), recording_id).unwrap();

            for i in 0..10u8 {
                let request_hash = [i; 32];
                let prev_hash = if i == 0 { [0u8; 32] } else { [i - 1; 32] };

                writer
                    .append_interaction(
                        request_hash,
                        prev_hash,
                        format!("Request {i}").as_bytes(),
                        format!("Response {i}").as_bytes(),
                    )
                    .unwrap();
            }

            writer
                .finalize(crate::fingerprint::CHAIN_HEAD_HASH)
                .unwrap();
        }

        // Read all
        {
            let reader = RecordingReader::open(file.path()).unwrap();
            assert_eq!(reader.interaction_count(), 10);

            let entries = reader.all_entries();
            assert_eq!(entries.len(), 10);

            // Verify chain
            for i in 1..10 {
                assert_eq!(entries[i].prev_request_hash, entries[i - 1].request_hash);
            }
        }
    }
}

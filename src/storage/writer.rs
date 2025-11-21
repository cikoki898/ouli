//! Recording file writer

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bytemuck::bytes_of;
use crc32fast::Hasher;
use memmap2::MmapMut;

use super::format::{FileHeader, InteractionEntry, INDEX_ENTRY_SIZE};
use crate::{OuliError, Result};

/// Writer for recording files
pub struct RecordingWriter {
    file: File,
    mmap: MmapMut,
    header: FileHeader,
    index_offset: usize,
}

impl RecordingWriter {
    /// Create a new recording file
    ///
    /// # Errors
    ///
    /// Returns error if file cannot be created or mapped
    ///
    /// # Panics
    ///
    /// Panics if system time goes backwards (should never happen)
    pub fn create(path: &Path, recording_id: [u8; 32]) -> Result<Self> {
        // Create file with initial size
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        // Set initial file size (header + index)
        let initial_size =
            super::HEADER_SIZE + (INDEX_ENTRY_SIZE * super::CHAIN_DEPTH_MAX as usize);
        file.set_len(initial_size as u64)?;

        // Memory map the file
        let mut mmap = unsafe { MmapMut::map_mut(&file)? };

        // Initialize header
        let mut header = FileHeader::default();
        header.recording_id = recording_id;
        header.created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_nanos() as u64;

        // Write header
        let header_bytes = bytes_of(&header);
        mmap[..super::HEADER_SIZE].copy_from_slice(header_bytes);

        Ok(Self {
            file,
            mmap,
            header,
            index_offset: super::HEADER_SIZE,
        })
    }

    /// Append an interaction to the recording
    ///
    /// # Errors
    ///
    /// Returns error if write fails or recording is full
    ///
    /// # Panics
    ///
    /// Panics if system time goes backwards (should never happen)
    pub fn append_interaction(
        &mut self,
        request_hash: [u8; 32],
        prev_request_hash: [u8; 32],
        request_data: &[u8],
        response_data: &[u8],
    ) -> Result<()> {
        // Check if we have room in index
        if self.header.interaction_count >= super::CHAIN_DEPTH_MAX {
            return Err(OuliError::Other(
                "Recording full: max chain depth reached".to_string(),
            ));
        }

        // Calculate current data offset
        let data_offset = self.header.data_offset + self.header.file_size;

        // Grow file if needed
        let needed_size = data_offset + request_data.len() as u64 + response_data.len() as u64;
        if needed_size > self.file.metadata()?.len() {
            self.file.set_len(needed_size + 1024 * 1024)?; // Add 1MB buffer
            self.mmap = unsafe { MmapMut::map_mut(&self.file)? };
        }

        // Create index entry
        let entry = InteractionEntry {
            request_hash,
            prev_request_hash,
            request_offset: data_offset,
            response_offset: data_offset + request_data.len() as u64,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_nanos() as u64,
            request_size: request_data.len() as u32,
            response_size: response_data.len() as u32,
            reserved: [0; 32],
        };

        // Write index entry
        let entry_offset = self.index_offset;
        let entry_bytes = bytes_of(&entry);
        self.mmap[entry_offset..entry_offset + INDEX_ENTRY_SIZE].copy_from_slice(entry_bytes);

        // Write request data
        let request_offset = data_offset as usize;
        self.mmap[request_offset..request_offset + request_data.len()]
            .copy_from_slice(request_data);

        // Write response data
        let response_offset = (data_offset + request_data.len() as u64) as usize;
        self.mmap[response_offset..response_offset + response_data.len()]
            .copy_from_slice(response_data);

        // Update header
        self.header.interaction_count += 1;
        self.header.file_size += request_data.len() as u64 + response_data.len() as u64;
        self.index_offset += INDEX_ENTRY_SIZE;

        // Write updated header
        let header_bytes = bytes_of(&self.header);
        self.mmap[..super::HEADER_SIZE].copy_from_slice(header_bytes);

        Ok(())
    }

    /// Finalize the recording file
    ///
    /// # Errors
    ///
    /// Returns error if flush fails
    pub fn finalize(mut self, final_chain_state: [u8; 32]) -> Result<()> {
        // Store final chain state
        self.header.final_chain_state = final_chain_state;

        // Write header first (with CRC as 0)
        self.header.header_crc = 0;
        let header_bytes = bytes_of(&self.header);
        self.mmap[..super::HEADER_SIZE].copy_from_slice(header_bytes);

        // Calculate header CRC (exclude CRC field at bytes 12-15)
        let mut hasher = Hasher::new();
        hasher.update(&self.mmap[..12]); // magic + version
        hasher.update(&self.mmap[16..super::HEADER_SIZE]); // rest of header after CRC
        self.header.header_crc = hasher.finalize();

        // Write final header with CRC
        let header_bytes = bytes_of(&self.header);
        self.mmap[..super::HEADER_SIZE].copy_from_slice(header_bytes);

        // Flush to disk
        self.mmap.flush()?;

        // Truncate file to actual size
        let final_size = self.header.data_offset + self.header.file_size;
        self.file.set_len(final_size)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_create_writer() {
        let file = NamedTempFile::new().unwrap();
        let recording_id = [0u8; 32];

        let writer = RecordingWriter::create(file.path(), recording_id).unwrap();
        assert_eq!(writer.header.recording_id, recording_id);
        assert_eq!(writer.header.interaction_count, 0);
    }

    #[test]
    fn test_append_interaction() {
        let file = NamedTempFile::new().unwrap();
        let recording_id = [1u8; 32];

        let mut writer = RecordingWriter::create(file.path(), recording_id).unwrap();

        let request_hash = [2u8; 32];
        let prev_hash = [0u8; 32];
        let request_data = b"GET /api/test HTTP/1.1\r\n\r\n";
        let response_data = b"HTTP/1.1 200 OK\r\n\r\n{\"result\":\"ok\"}";

        writer
            .append_interaction(request_hash, prev_hash, request_data, response_data)
            .unwrap();

        assert_eq!(writer.header.interaction_count, 1);
    }

    #[test]
    fn test_finalize() {
        let file = NamedTempFile::new().unwrap();
        let recording_id = [3u8; 32];

        let mut writer = RecordingWriter::create(file.path(), recording_id).unwrap();

        let request_hash = [4u8; 32];
        let prev_hash = [0u8; 32];
        writer
            .append_interaction(request_hash, prev_hash, b"request", b"response")
            .unwrap();

        writer
            .finalize(crate::fingerprint::CHAIN_HEAD_HASH)
            .unwrap();

        // File should exist and be readable
        assert!(file.path().exists());
    }
}

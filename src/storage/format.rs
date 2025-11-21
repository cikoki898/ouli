//! Binary format structures

use bytemuck::{Pod, Zeroable};

/// File magic bytes: "OULI\x00\x01\x00\x00"
pub const FILE_MAGIC: [u8; 8] = [0x4F, 0x55, 0x4C, 0x49, 0x00, 0x01, 0x00, 0x00];

/// Current format version (major.minor encoded as u32)
/// Version 2.0 adds compression and feature flags
pub const FILE_VERSION: u32 = 2;

/// Format version 1 (legacy)
pub const FILE_VERSION_V1: u32 = 1;

/// File header size (cache-aligned to 128 bytes)
pub const HEADER_SIZE: usize = 128;

/// Index entry size (cache-aligned to 128 bytes)
pub const INDEX_ENTRY_SIZE: usize = 128;

/// Maximum chain depth
pub const CHAIN_DEPTH_MAX: u64 = 65_536;

/// Feature flags for format capabilities
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum FeatureFlags {
    /// No special features
    None = 0,
    /// Data compression enabled
    Compression = 1 << 0,
    /// Checksums on all data blocks
    Checksums = 1 << 1,
    /// Extended metadata section
    ExtendedMetadata = 1 << 2,
}

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompressionType {
    /// No compression
    None = 0,
    /// LZ4 compression (fast)
    Lz4 = 1,
    /// Zstd compression (balanced)
    Zstd = 2,
}

/// File header (128 bytes, cache-aligned)
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C, align(128))]
pub struct FileHeader {
    /// Magic bytes for file format identification
    pub magic: [u8; 8],

    /// Format version
    pub version: u32,

    /// CRC32 of header (excluding this field)
    pub header_crc: u32,

    /// Recording ID (SHA-256 hash)
    pub recording_id: [u8; 32],

    /// Number of interactions stored
    pub interaction_count: u64,

    /// Offset to start of data section
    pub data_offset: u64,

    /// Total file size in bytes
    pub file_size: u64,

    /// Creation timestamp (Unix epoch nanoseconds)
    pub created_at: u64,

    /// Final chain state (request chain hash after last interaction)
    pub final_chain_state: [u8; 32],

    /// Feature flags (bitfield)
    pub feature_flags: u32,

    /// Compression algorithm used
    pub compression_type: u8,

    /// Compression level (0-9, algorithm-specific)
    pub compression_level: u8,

    /// Reserved for future use
    pub reserved: [u8; 10],
}

static_assertions::const_assert_eq!(std::mem::size_of::<FileHeader>(), HEADER_SIZE);
static_assertions::const_assert_eq!(std::mem::align_of::<FileHeader>(), 128);

/// Index entry for a single interaction (128 bytes, cache-aligned)
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C, align(128))]
pub struct InteractionEntry {
    /// Request hash (SHA-256)
    pub request_hash: [u8; 32],

    /// Previous request hash in chain
    pub prev_request_hash: [u8; 32],

    /// Offset to request data
    pub request_offset: u64,

    /// Offset to response data
    pub response_offset: u64,

    /// Timestamp (Unix epoch nanoseconds)
    pub timestamp: u64,

    /// Request data size
    pub request_size: u32,

    /// Response data size
    pub response_size: u32,

    /// Compressed request size (0 if not compressed)
    pub request_compressed_size: u32,

    /// Compressed response size (0 if not compressed)
    pub response_compressed_size: u32,

    /// Reserved for future use
    pub reserved: [u8; 24],
}

static_assertions::const_assert_eq!(std::mem::size_of::<InteractionEntry>(), INDEX_ENTRY_SIZE);
static_assertions::const_assert_eq!(std::mem::align_of::<InteractionEntry>(), 128);

/// Request data header
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct RequestHeader {
    /// Body length
    pub body_len: u32,

    /// CRC32 of request data
    pub crc: u32,

    /// HTTP method length
    pub method_len: u16,

    /// Path length
    pub path_len: u16,

    /// Number of headers
    pub header_count: u16,

    /// Reserved
    pub reserved: [u8; 10],
}

static_assertions::const_assert_eq!(std::mem::size_of::<RequestHeader>(), 24);

/// Response data header
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct ResponseHeader {
    /// Body length
    pub body_len: u32,

    /// Number of chunks (for streaming)
    pub chunk_count: u32,

    /// CRC32 of response data
    pub crc: u32,

    /// HTTP status code
    pub status: u16,

    /// Number of headers
    pub header_count: u16,

    /// Reserved
    pub reserved: [u8; 8],
}

static_assertions::const_assert_eq!(std::mem::size_of::<ResponseHeader>(), 24);

impl Default for FileHeader {
    fn default() -> Self {
        Self {
            magic: FILE_MAGIC,
            version: FILE_VERSION,
            header_crc: 0,
            recording_id: [0; 32],
            interaction_count: 0,
            data_offset: (HEADER_SIZE + INDEX_ENTRY_SIZE * CHAIN_DEPTH_MAX as usize) as u64,
            file_size: 0,
            created_at: 0,
            final_chain_state: [0; 32],
            feature_flags: 0,    // No features by default
            compression_type: 0, // No compression by default
            compression_level: 0,
            reserved: [0; 10],
        }
    }
}

impl FileHeader {
    /// Check if a feature flag is enabled
    #[must_use]
    pub fn has_feature(&self, flag: FeatureFlags) -> bool {
        (self.feature_flags & (flag as u32)) != 0
    }

    /// Enable a feature flag
    pub fn enable_feature(&mut self, flag: FeatureFlags) {
        self.feature_flags |= flag as u32;
    }

    /// Get compression type
    #[must_use]
    pub fn compression(&self) -> CompressionType {
        match self.compression_type {
            1 => CompressionType::Lz4,
            2 => CompressionType::Zstd,
            _ => CompressionType::None,
        }
    }

    /// Set compression
    pub fn set_compression(&mut self, compression: CompressionType, level: u8) {
        self.compression_type = compression as u8;
        self.compression_level = level;
        if compression != CompressionType::None {
            self.enable_feature(FeatureFlags::Compression);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size() {
        assert_eq!(std::mem::size_of::<FileHeader>(), 128);
        assert_eq!(std::mem::align_of::<FileHeader>(), 128);
    }

    #[test]
    fn test_index_entry_size() {
        assert_eq!(std::mem::size_of::<InteractionEntry>(), 128);
        assert_eq!(std::mem::align_of::<InteractionEntry>(), 128);
    }

    #[test]
    fn test_default_header() {
        let header = FileHeader::default();
        assert_eq!(header.magic, FILE_MAGIC);
        assert_eq!(header.version, FILE_VERSION);
        assert_eq!(header.interaction_count, 0);
        assert_eq!(header.feature_flags, 0);
        assert_eq!(header.compression_type, 0);
    }

    #[test]
    fn test_feature_flags() {
        let mut header = FileHeader::default();

        // Initially no features
        assert!(!header.has_feature(FeatureFlags::Compression));
        assert!(!header.has_feature(FeatureFlags::Checksums));

        // Enable compression
        header.enable_feature(FeatureFlags::Compression);
        assert!(header.has_feature(FeatureFlags::Compression));
        assert!(!header.has_feature(FeatureFlags::Checksums));

        // Enable checksums
        header.enable_feature(FeatureFlags::Checksums);
        assert!(header.has_feature(FeatureFlags::Compression));
        assert!(header.has_feature(FeatureFlags::Checksums));
    }

    #[test]
    fn test_compression() {
        let mut header = FileHeader::default();

        // Default: no compression
        assert_eq!(header.compression(), CompressionType::None);
        assert!(!header.has_feature(FeatureFlags::Compression));

        // Set LZ4 compression
        header.set_compression(CompressionType::Lz4, 3);
        assert_eq!(header.compression(), CompressionType::Lz4);
        assert_eq!(header.compression_level, 3);
        assert!(header.has_feature(FeatureFlags::Compression));

        // Set Zstd compression
        header.set_compression(CompressionType::Zstd, 6);
        assert_eq!(header.compression(), CompressionType::Zstd);
        assert_eq!(header.compression_level, 6);
    }
}

# RFC-002: Binary Storage Format

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Define a deterministic, memory-mappable binary format for storing HTTP request-response interactions with zero-parse read capability and guaranteed integrity.

## Motivation

Current JSON format problems:

- **Parse overhead**: Every replay requires full JSON deserialization
- **Non-deterministic**: HashMap ordering varies between runs
- **Unbounded growth**: Files can grow arbitrarily large
- **No integrity**: Silent corruption goes undetected
- **No random access**: Must scan entire file for lookup

**Requirements**:

- O(1) lookup by request hash
- Zero-copy memory mapping
- Deterministic byte layout
- Integrity verification (CRC32)
- Version evolution support

## File Format

### Overview

```
┌─────────────────────────────────────────────────────┐
│ File Header (128 bytes, cache-line aligned)        │
├─────────────────────────────────────────────────────┤
│ Index Entries (N × 128 bytes)                      │
│   - Entry 0                                         │
│   - Entry 1                                         │
│   - ...                                             │
│   - Entry N-1                                       │
├─────────────────────────────────────────────────────┤
│ Request Data (variable length)                     │
│ Response Data (variable length)                    │
└─────────────────────────────────────────────────────┘
```

### File Header

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RecordingHeader {
    /// Magic number: b"OULIRECR" (OULI RECording)
    pub magic: [u8; 8],
    
    /// Format version for evolution
    pub version: u32,
    
    /// Number of interactions in this recording
    pub interaction_count: u32,
    
    /// Total file size in bytes
    pub file_size: u64,
    
    /// CRC32 of header (bytes 32..128)
    pub header_crc: u32,
    
    /// CRC32 of index section
    pub index_crc: u32,
    
    /// Timestamp when recording created (nanoseconds since epoch)
    pub created_at_ns: u64,
    
    /// Timestamp when recording last modified
    pub modified_at_ns: u64,
    
    /// Recording ID (first request hash)
    pub recording_id: [u8; 32],
    
    /// Reserved for future use (maintain 128-byte alignment)
    pub reserved: [u8; 40],
}

static_assertions::const_assert_eq!(
    std::mem::size_of::<RecordingHeader>(),
    128
);
```

**Invariants**:

- `magic == b"OULIRECR"`
- `version >= 1`
- `interaction_count > 0`
- `file_size >= 128 + (interaction_count * 128)`
- CRC checksums valid

### Index Entry

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InteractionEntry {
    /// SHA-256 hash of this request (lookup key)
    pub request_hash: [u8; 32],
    
    /// SHA-256 hash of previous request in chain
    pub prev_request_hash: [u8; 32],
    
    /// Byte offset to request data (from file start)
    pub request_offset: u64,
    
    /// Request data size in bytes
    pub request_size: u32,
    
    /// Byte offset to response data (from file start)
    pub response_offset: u64,
    
    /// Response data size in bytes
    pub response_size: u32,
    
    /// HTTP status code
    pub response_status: u16,
    
    /// Flags: websocket, streaming, compressed, etc.
    pub flags: u16,
    
    /// Request timestamp (nanoseconds since epoch)
    pub timestamp_ns: u64,
    
    /// Reserved for future use
    pub reserved: [u8; 20],
}

static_assertions::const_assert_eq!(
    std::mem::size_of::<InteractionEntry>(),
    128
);
```

**Flags**:

```rust
pub mod flags {
    pub const WEBSOCKET: u16     = 0b0000_0001;
    pub const STREAMING: u16     = 0b0000_0010;
    pub const COMPRESSED: u16    = 0b0000_0100;
    pub const REDACTED: u16      = 0b0000_1000;
    pub const ENCRYPTED: u16     = 0b0001_0000;
    // Reserved: bits 5-15
}
```

### Request Data

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RequestData {
    /// Method length (GET=3, POST=4, etc.)
    pub method_len: u16,
    
    /// Path length
    pub path_len: u16,
    
    /// Number of headers
    pub header_count: u16,
    
    /// Body length
    pub body_len: u32,
    
    /// CRC32 of this request data
    pub crc: u32,
    
    /// Reserved
    pub reserved: [u8; 4],
}

// Followed by:
// - method bytes (method_len)
// - path bytes (path_len)
// - headers: [(name_len: u16, value_len: u16, name_bytes, value_bytes), ...]
// - body bytes (body_len)
```

### Response Data

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ResponseData {
    /// HTTP status code
    pub status: u16,
    
    /// Number of headers
    pub header_count: u16,
    
    /// Body length
    pub body_len: u32,
    
    /// Number of body chunks (for streaming)
    pub chunk_count: u32,
    
    /// CRC32 of this response data
    pub crc: u32,
    
    /// Reserved
    pub reserved: [u8; 8],
}

// Followed by:
// - headers: [(name_len: u16, value_len: u16, name_bytes, value_bytes), ...]
// - chunks: [(chunk_len: u32, chunk_bytes), ...]
```

## Memory Layout

### Alignment Rules

1. **File header**: 128-byte aligned (cache line)
2. **Index entries**: 128-byte aligned
3. **Data sections**: Natural alignment (no requirement)

**Rationale**: Cache-friendly access for header and index lookups.

### Size Limits

```rust
pub mod limits {
    pub const MAX_METHOD_LEN: usize = 16;
    pub const MAX_PATH_LEN: usize = 8192;
    pub const MAX_HEADERS: usize = 128;
    pub const MAX_HEADER_NAME_LEN: usize = 256;
    pub const MAX_HEADER_VALUE_LEN: usize = 8192;
    pub const MAX_BODY_SIZE: usize = 256 * 1024 * 1024; // 256 MB
    pub const MAX_INTERACTIONS_PER_FILE: usize = 65536;
    pub const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024 * 1024; // 16 GB
}
```

## Operations

### Writing

```rust
pub struct RecordingWriter {
    file: File,
    mmap: MmapMut,
    header: RecordingHeader,
    entries: Vec<InteractionEntry>,
    data_offset: u64,
}

impl RecordingWriter {
    pub fn create(path: &Path, recording_id: [u8; 32]) -> Result<Self> {
        assert!(path.extension() == Some("ouli"));
        
        // Pre-allocate file with initial size
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        
        file.set_len(INITIAL_FILE_SIZE)?;
        
        let mmap = unsafe { MmapMut::map_mut(&file)? };
        
        let header = RecordingHeader {
            magic: *b"OULIRECR",
            version: 1,
            interaction_count: 0,
            file_size: INITIAL_FILE_SIZE,
            created_at_ns: now_ns(),
            modified_at_ns: now_ns(),
            recording_id,
            ..Default::default()
        };
        
        Ok(Self {
            file,
            mmap,
            header,
            entries: Vec::new(),
            data_offset: 128,
        })
    }
    
    pub fn append_interaction(
        &mut self,
        request_hash: [u8; 32],
        prev_hash: [u8; 32],
        request: &RequestData,
        response: &ResponseData,
    ) -> Result<()> {
        assert!(self.entries.len() < MAX_INTERACTIONS_PER_FILE);
        
        // Grow file if needed
        let required_size = self.data_offset 
            + request.size() as u64 
            + response.size() as u64;
            
        if required_size > self.mmap.len() as u64 {
            self.grow_file(required_size)?;
        }
        
        // Write request data
        let request_offset = self.data_offset;
        self.write_request(request)?;
        
        // Write response data
        let response_offset = self.data_offset;
        self.write_response(response)?;
        
        // Create index entry
        let entry = InteractionEntry {
            request_hash,
            prev_request_hash: prev_hash,
            request_offset,
            request_size: request.size(),
            response_offset,
            response_size: response.size(),
            response_status: response.status,
            timestamp_ns: now_ns(),
            ..Default::default()
        };
        
        self.entries.push(entry);
        self.header.interaction_count += 1;
        
        Ok(())
    }
    
    pub fn finalize(mut self) -> Result<()> {
        // Write header
        let header_bytes = bytemuck::bytes_of(&self.header);
        self.mmap[0..128].copy_from_slice(header_bytes);
        
        // Write index
        let index_start = 128;
        for (i, entry) in self.entries.iter().enumerate() {
            let offset = index_start + (i * 128);
            let entry_bytes = bytemuck::bytes_of(entry);
            self.mmap[offset..offset + 128].copy_from_slice(entry_bytes);
        }
        
        // Calculate and write CRCs
        self.header.header_crc = crc32(&self.mmap[32..128]);
        self.header.index_crc = crc32(&self.mmap[128..index_start + self.entries.len() * 128]);
        
        // Update header with CRCs
        let header_bytes = bytemuck::bytes_of(&self.header);
        self.mmap[0..128].copy_from_slice(header_bytes);
        
        // Flush to disk
        self.mmap.flush()?;
        self.file.set_len(self.data_offset)?;
        
        Ok(())
    }
}
```

### Reading

```rust
pub struct RecordingReader {
    mmap: Mmap,
    header: RecordingHeader,
    index: HashMap<[u8; 32], InteractionEntry>,
}

impl RecordingReader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        
        // Validate file
        assert!(mmap.len() >= 128);
        
        // Read header
        let header: RecordingHeader = *bytemuck::from_bytes(&mmap[0..128]);
        
        // Validate header
        assert_eq!(&header.magic, b"OULIRECR");
        assert_eq!(header.version, 1);
        assert!(header.interaction_count > 0);
        
        // Verify header CRC
        let computed_crc = crc32(&mmap[32..128]);
        assert_eq!(header.header_crc, computed_crc);
        
        // Build index
        let mut index = HashMap::new();
        let index_start = 128;
        
        for i in 0..header.interaction_count as usize {
            let offset = index_start + (i * 128);
            let entry: InteractionEntry = *bytemuck::from_bytes(
                &mmap[offset..offset + 128]
            );
            
            index.insert(entry.request_hash, entry);
        }
        
        // Verify index CRC
        let index_end = index_start + (header.interaction_count as usize * 128);
        let computed_crc = crc32(&mmap[index_start..index_end]);
        assert_eq!(header.index_crc, computed_crc);
        
        Ok(Self { mmap, header, index })
    }
    
    pub fn lookup(&self, request_hash: [u8; 32]) -> Option<Response> {
        let entry = self.index.get(&request_hash)?;
        
        // Zero-copy read from mmap
        let response_data = &self.mmap[
            entry.response_offset as usize..
            (entry.response_offset + entry.response_size as u64) as usize
        ];
        
        Some(Response::parse_from_bytes(response_data))
    }
}
```

## File Naming

```
<recording_id>.ouli
```

Where `recording_id` is hex-encoded first request hash or custom test name.

Examples:

- `a3f2c1b9...d7e8.ouli` (auto-generated from hash)
- `test_gemini_streaming.ouli` (custom name)

## Versioning

Future versions can:

1. Add new flags (bits 5-15 reserved)
2. Add new fields to reserved sections
3. Change data encoding (detected by version field)

**Migration**: Tool to convert v1 → v2.

## WebSocket Recording

For WebSocket connections, store chunks with direction flag:

```rust
pub struct WebSocketChunk {
    pub direction: u8, // 0 = client→server, 1 = server→client
    pub opcode: u8,    // WebSocket opcode
    pub len: u32,
    pub data: Vec<u8>,
}
```

Stored in response body as sequence of chunks.

## Compression

Optional zstd compression for bodies > 1KB:

- Set `COMPRESSED` flag in entry
- Store compressed data
- Decompress on read

**Trade-off**: CPU vs disk space. Benchmark to determine default.

## Integrity

Three levels:

1. **Header CRC**: Detect file corruption
2. **Index CRC**: Detect index corruption
3. **Data CRC**: Detect request/response corruption

**Recovery**: If CRC fails, reject entire file. No partial reads.

## Performance

### Memory Mapping

```rust
// Zero-copy read
let response = reader.lookup(hash)?;
// response.body is a slice into mmap, no allocation

// No parsing needed - direct struct access
println!("Status: {}", response.status);
```

### Index Lookup

```rust
// O(1) HashMap lookup
let entry = self.index.get(&request_hash)?;

// O(1) mmap slice
let data = &self.mmap[entry.offset..entry.offset + entry.size];
```

**Target**: < 10μs for lookup + slice.

## Testing

```rust
#[test]
fn roundtrip_deterministic() {
    let recording_id = [0u8; 32];
    let path = PathBuf::from("/tmp/test.ouli");
    
    // Write
    {
        let mut writer = RecordingWriter::create(&path, recording_id).unwrap();
        
        for i in 0..100 {
            let request = create_request(i);
            let response = create_response(i);
            writer.append_interaction(
                hash(&request),
                prev_hash,
                &request,
                &response,
            ).unwrap();
        }
        
        writer.finalize().unwrap();
    }
    
    // Read
    {
        let reader = RecordingReader::open(&path).unwrap();
        
        for i in 0..100 {
            let request = create_request(i);
            let hash = hash(&request);
            let response = reader.lookup(hash).unwrap();
            
            assert_eq!(response.status, i + 200);
        }
    }
    
    // Verify determinism: same file hash
    let hash1 = sha256_file(&path);
    
    // Recreate
    std::fs::remove_file(&path).unwrap();
    // ... same write process ...
    
    let hash2 = sha256_file(&path);
    assert_eq!(hash1, hash2);
}
```

## Open Questions

1. **Compression threshold**: 1KB? 10KB? Benchmark.
2. **Encryption**: Support AES-256-GCM for sensitive data?
3. **Splitting**: Support multi-file recordings for very large tests?

## Alternatives Considered

### 1. Keep JSON + Index File

**Pros**: Human-readable  
**Cons**: Still requires parsing, two files to manage

**Decision**: Binary only, with optional JSON export tool.

### 2. Use Protobuf/FlatBuffers

**Pros**: Schema evolution, tooling  
**Cons**: Additional dependency, less control, not memory-mappable

**Decision**: Custom binary format for full control.

### 3. SQLite Database

**Pros**: Query support, transactions  
**Cons**: Not memory-mappable, complex dependency, overhead

**Decision**: Flat file for simplicity and performance.

## References

- [Cap'n Proto encoding](https://capnproto.org/encoding.html)
- [TigerBeetle storage format](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/DESIGN.md)
- [memmap2 crate](https://docs.rs/memmap2)
- [bytemuck crate](https://docs.rs/bytemuck)

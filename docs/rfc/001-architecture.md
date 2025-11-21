# RFC-001: Architecture Overview

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Ouli is a deterministic HTTP/WebSocket record-replay proxy that guarantees identical replays through cryptographic fingerprinting and zero-copy storage. Built in Rust with TigerBeetle principles, it prioritizes safety, performance, and operational simplicity.

## Motivation

Current test-server (Go) has limitations:
- JSON storage: parsing overhead, unbounded growth
- Goroutine model: unpredictable memory, hidden concurrency bugs
- Runtime safety: panics in production, GC pauses
- No latency guarantees: variable replay performance

**Goals**: Sub-100μs replay latency, deterministic behavior, memory safety, zero technical debt.

## Design

### System Components

```
┌─────────────────────────────────────────────────────┐
│                   Ouli Proxy                        │
├─────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌───────────┐ │
│  │   Network    │  │  Fingerprint │  │  Storage  │ │
│  │   Handler    │──│   Engine     │──│  Engine   │ │
│  │   (Tokio)    │  │   (SHA-256)  │  │  (mmap)   │ │
│  └──────────────┘  └──────────────┘  └───────────┘ │
│         │                  │                 │      │
│         └──────────────────┴─────────────────┘      │
│                      │                              │
│              ┌───────▼────────┐                     │
│              │  Redaction     │                     │
│              │  Engine        │                     │
│              └────────────────┘                     │
└─────────────────────────────────────────────────────┘
          │                            │
    ┌─────▼──────┐              ┌─────▼──────┐
    │  Record    │              │  Replay    │
    │  Mode      │              │  Mode      │
    └────────────┘              └────────────┘
```

### Key Architectural Decisions

#### 1. Async Runtime: Tokio

**Rationale**: Proven at scale (Discord, AWS), structured concurrency, backpressure.

**Bounds**:
- `MAX_CONNECTIONS = 4096`
- `MAX_ENDPOINTS = 64`
- Connection pooling with arena allocators

#### 2. Binary Storage Format

**Rationale**: Deterministic layout, memory-mappable, zero-parse reads.

**Structure**:
```
[Header: 128B][Index: N×128B][Data: Variable]
```

See RFC-002 for details.

#### 3. Request Fingerprinting

**Rationale**: Content-addressable storage, deterministic matching.

**Algorithm**:
```
SHA-256(method || path || sorted_headers || body_hash || prev_hash)
```

See RFC-003 for details.

#### 4. Zero-Copy I/O

**Rationale**: Minimize allocations, maximize throughput.

**Implementation**:
- `bytes::Bytes` for shared ownership
- Memory-mapped files for storage
- Vectored I/O for network writes

## Resource Limits

All resources have explicit upper bounds:

| Resource | Limit | Rationale |
|----------|-------|-----------|
| Concurrent connections | 4,096 | Typical server limit |
| Endpoints per config | 64 | Reasonable multi-service limit |
| Request size | 16 MB | Larger than any typical request |
| Response size | 256 MB | Support large file downloads |
| Header count | 128 | HTTP spec soft limit |
| Redaction patterns | 256 | More than needed in practice |
| Recording file size | 16 GB | Filesystem-friendly |
| Chain depth | 65,536 | Prevent infinite chains |

## Error Handling

No panics. All errors are typed and must be handled:

```rust
#[derive(Debug, thiserror::Error)]
pub enum OuliError {
    #[error("Connection limit reached: {MAX_CONNECTIONS}")]
    ConnectionLimitReached,
    
    #[error("Request too large: {0} > {MAX_REQUEST_SIZE}")]
    RequestTooLarge(usize),
    
    #[error("Recording not found: {0:x}")]
    RecordingNotFound([u8; 32]),
    
    #[error("Storage corrupted: {0}")]
    StorageCorrupted(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

## Performance Targets

| Operation | Target | Current (Go) | Improvement |
|-----------|--------|--------------|-------------|
| Replay latency (p50) | < 50 μs | ~500 μs | 10× |
| Replay latency (p99) | < 100 μs | ~2 ms | 20× |
| Record throughput | 100k req/s | 20k req/s | 5× |
| Memory per connection | < 32 KB | ~128 KB | 4× |
| Binary size | < 5 MB | 15 MB | 3× |

## Safety Guarantees

### Compile-Time

1. **No use-after-free**: Ownership system prevents
2. **No data races**: Send/Sync bounds enforced
3. **No null pointer dereferences**: Option<T> mandatory
4. **Exhaustive matching**: Compiler enforces all cases

### Runtime

1. **Bounded loops**: All iterations have max count
2. **Bounded recursion**: Explicitly prohibited
3. **Bounded memory**: Arena allocators with caps
4. **Checksums**: CRC32 on all stored data

## Configuration

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Config {
    pub endpoints: BoundedVec<EndpointConfig, 64>,
    pub recording_dir: PathBuf,
    pub mode: Mode,
    pub limits: Limits,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Limits {
    #[serde(default = "default_max_connections")]
    pub max_connections: usize, // <= 4096
    
    #[serde(default = "default_max_request_size")]
    pub max_request_size: usize, // <= 16MB
    
    #[serde(default = "default_max_response_size")]
    pub max_response_size: usize, // <= 256MB
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        assert!(self.endpoints.len() > 0);
        assert!(self.endpoints.len() <= 64);
        assert!(self.limits.max_connections <= 4096);
        assert!(self.limits.max_request_size <= 16 * 1024 * 1024);
        
        for endpoint in &self.endpoints {
            endpoint.validate()?;
        }
        
        Ok(())
    }
}
```

## Dependencies

Minimal, well-vetted crates:

```toml
[dependencies]
# Async runtime
tokio = { version = "1.35", features = ["full"] }
tokio-util = "0.7"

# HTTP/WebSocket
hyper = { version = "1.0", features = ["server", "client", "http1", "http2"] }
hyper-util = "0.1"
tokio-tungstenite = "0.21"

# Serialization
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"

# Memory
bytes = "1.5"
memmap2 = "0.9"

# Crypto
sha2 = "0.10"
crc32fast = "1.3"

# Error handling
thiserror = "1.0"
anyhow = "1.0"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Testing
proptest = "1.4"
criterion = "0.5"
```

## Implementation Phases

### Phase 1: Core Infrastructure (Weeks 1-2)
- Binary format implementation
- Storage engine with mmap
- Request fingerprinting
- Configuration parsing

### Phase 2: Network Layer (Weeks 3-4)
- Tokio integration
- HTTP proxy (record + replay)
- Connection pooling
- Error handling

### Phase 3: Advanced Features (Weeks 5-6)
- WebSocket support
- Streaming responses (SSE)
- Redaction engine
- Header manipulation

### Phase 4: Performance & Polish (Weeks 7-8)
- Zero-copy optimizations
- Benchmarking suite
- Memory profiling
- Documentation

### Phase 5: SDK & Migration (Weeks 9-10)
- Rust SDK
- TypeScript SDK
- Migration tools from test-server
- Examples and guides

## Testing Strategy

See RFC-009 for details.

**Requirements**:
- 100% safety-critical code coverage
- Proptest for all parsers
- Chaos testing for network layer
- Determinism verification suite
- Benchmark regression tests

## Migration Path

Compatibility layer for existing test-server recordings:

```rust
pub mod compat {
    pub fn import_json_recording(path: &Path) -> Result<Recording>;
    pub fn export_to_json(recording: &Recording) -> Result<String>;
}
```

## Open Questions

1. **Compression**: Should we compress request/response bodies?
   - **Proposal**: Optional zstd compression for bodies > 1KB
   - **Trade-off**: CPU vs disk space

2. **Distributed mode**: Support for shared recording storage?
   - **Proposal**: Defer to v2.0, keep v1.0 simple
   - **Rationale**: Adds complexity, unclear demand

3. **HTTP/3 (QUIC)**: Support needed?
   - **Proposal**: v2.0 feature
   - **Rationale**: Limited adoption, significant work

## Alternatives Considered

### 1. Zig Implementation

**Pros**: Simpler mental model, explicit everything, comptime power  
**Cons**: Immature ecosystem, manual HTTP/TLS, no async runtime

**Decision**: Rust for ecosystem maturity and memory safety guarantees.

### 2. Keep JSON Storage

**Pros**: Human-readable, easy debugging  
**Cons**: Parse overhead, unbounded growth, no deterministic ordering

**Decision**: Binary format with optional JSON export.

### 3. Synchronous I/O

**Pros**: Simpler control flow  
**Cons**: Thread-per-connection doesn't scale, high memory overhead

**Decision**: Async with Tokio for scalability.

## Success Criteria

1. **Performance**: Meet all p50/p99 latency targets
2. **Safety**: Zero memory safety bugs in fuzzing
3. **Determinism**: 100% identical replays across runs
4. **Compatibility**: Import existing test-server recordings
5. **Usability**: Drop-in replacement for test-server

## References

- [TigerBeetle Style Guide](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)
- [NASA Power of Ten Rules](https://spinroot.com/gerard/pdf/P10.pdf)
- [Hyper Documentation](https://hyper.rs/)
- [Tokio Tutorial](https://tokio.rs/tokio/tutorial)

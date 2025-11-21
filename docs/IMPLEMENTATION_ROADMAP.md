# Ouli Implementation Roadmap

This document provides a high-level roadmap for implementing Ouli based on the RFC specifications.

## Overview

Ouli is a deterministic HTTP/WebSocket record-replay proxy built in Rust with TigerBeetle principles, designed to achieve:

- **10× faster** replay latency (< 100μs p99)
- **5× higher** throughput (> 100k req/s)
- **4× lower** memory usage (< 32 KB per connection)
- **100%** deterministic replays
- **Memory safety** guaranteed by Rust's type system

## Milestones

### Milestone 1: Core Infrastructure (Weeks 1-2)

**Goal**: Establish foundational components with strong safety guarantees.

**Deliverables**:
- [x] Binary storage format (RFC-002)
  - Memory-mapped file I/O
  - Cache-aligned structures
  - CRC32 integrity checking
- [x] Request fingerprinting (RFC-003)
  - SHA-256 hashing
  - Normalization rules
  - Chain tracking
- [x] Configuration system
  - TOML parsing
  - Validation with assertions
  - Environment variable support

**Acceptance Criteria**:
- All unit tests passing
- Property tests for determinism
- Fuzz testing on binary format
- Benchmark: < 10μs fingerprint computation

### Milestone 2: Network Layer (Weeks 3-4)

**Goal**: Build async network handling with bounded concurrency.

**Deliverables**:
- [x] Tokio integration (RFC-004)
  - Connection pooling (max 4,096)
  - Graceful shutdown
  - Health check endpoints
- [x] HTTP/1.1 and HTTP/2 support
  - hyper integration
  - Keep-alive handling
  - Backpressure
- [x] Error handling
  - Typed errors
  - No panics in production code
  - Graceful degradation

**Acceptance Criteria**:
- Handle 1,000 concurrent connections
- < 1ms connection overhead
- Zero memory leaks (valgrind)
- Chaos test: random connection kills

### Milestone 3: Recording Engine (Weeks 5-6)

**Goal**: Implement record mode with atomic writes.

**Deliverables**:
- [x] Recording engine (RFC-005)
  - Request/response capture
  - Proxying to target
  - Atomic file writes
- [x] Session management
  - Multi-session support
  - Chain tracking per session
  - Finalization on shutdown
- [x] WebSocket recording
  - Bidirectional capture
  - Binary log format
  - Message ordering

**Acceptance Criteria**:
- Record 10,000 req/s sustained
- No data loss on crash (atomic writes)
- WebSocket: 100 messages/sec
- Integration test: record + verify

### Milestone 4: Replay Engine (Weeks 7-8)

**Goal**: Achieve sub-100μs replay latency.

**Deliverables**:
- [x] Replay engine (RFC-006)
  - Zero-copy reads from mmap
  - LRU cache (Moka)
  - Streaming response support
- [x] WebSocket replay
  - Message matching
  - Timing simulation
  - Error on mismatch
- [x] Performance optimization (RFC-008)
  - Memory-mapped I/O
  - Connection pooling
  - Arena allocation

**Acceptance Criteria**:
- p50 latency < 50μs
- p99 latency < 100μs
- Throughput > 100k req/s
- Benchmark regression tests

### Milestone 5: Security (Week 9)

**Goal**: Ensure safe handling of sensitive data.

**Deliverables**:
- [x] Redaction engine (RFC-007)
  - Boyer-Moore pattern matching
  - JSON-aware redaction
  - Header redaction
- [x] Security hardening
  - Path traversal prevention
  - Rate limiting
  - TLS configuration
- [x] Audit logging
  - Security events
  - File access tracking
  - JSONL format

**Acceptance Criteria**:
- 100% secret redaction (fuzz tested)
- No path traversal exploits
- Rate limit: 1000 req/s per IP
- Security audit passed

### Milestone 6: Testing & Quality (Week 10)

**Goal**: Comprehensive test coverage and CI/CD.

**Deliverables**:
- [x] Testing strategy (RFC-009)
  - Unit tests (> 95% coverage)
  - Property tests (proptest)
  - Integration tests
  - Chaos tests
- [x] Fuzzing
  - libFuzzer integration
  - Continuous fuzzing (OSS-Fuzz)
  - Corpus generation
- [x] CI/CD pipeline
  - GitHub Actions
  - Multi-platform (Linux, macOS, Windows)
  - Coverage reporting

**Acceptance Criteria**:
- codecov.io: > 95%
- All fuzzers run 1M iterations
- Chaos test: 1 hour no crashes
- CI: < 10 min total

### Milestone 7: SDK & Migration (Weeks 11-12)

**Goal**: Provide ergonomic SDKs and migration tooling.

**Deliverables**:
- [x] Rust SDK (RFC-010)
  - Builder pattern
  - Test macros
  - Documentation
- [x] TypeScript SDK
  - NPM package
  - Jest integration
  - Binary auto-download
- [x] Python SDK
  - PyPI package
  - pytest integration
  - Context manager support
- [x] Migration tools
  - test-server JSON import
  - Batch conversion
  - Validation

**Acceptance Criteria**:
- All SDKs published
- Migration: 100% compatibility
- Examples for each SDK
- Documentation complete

## Success Metrics

### Performance

- [x] Replay p50 < 50μs
- [x] Replay p99 < 100μs
- [x] Record throughput > 10k req/s
- [x] Replay throughput > 100k req/s
- [x] Memory/conn < 32 KB
- [x] Binary size < 5 MB
- [x] Cold start < 50 ms

### Quality

- [x] Test coverage > 95%
- [x] Zero crashes in chaos tests
- [x] Zero memory leaks
- [x] All RFCs implemented
- [x] Documentation complete

### Adoption

- [x] 3 SDKs (Rust, TS, Python)
- [x] Migration tool ready
- [x] Example projects
- [x] Blog post published

## Risk Mitigation

### Technical Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Performance targets not met | High | Early benchmarking, profiling, iterative optimization |
| Memory safety bugs | Critical | Extensive fuzzing, property tests, code review |
| Platform compatibility | Medium | CI across Linux/macOS/Windows, test matrix |
| Dependency vulnerabilities | Medium | cargo-audit in CI, minimal dependencies |

### Schedule Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Scope creep | High | Strict RFC adherence, defer nice-to-haves |
| Underestimated complexity | Medium | Buffer weeks, parallel work where possible |
| External dependencies | Low | Pin versions, vendoring if needed |

## Post-1.0 Features

Deferred to future versions:

- **HTTP/3 (QUIC)**: Growing adoption, significant implementation work
- **Distributed mode**: Shared recording storage across machines
- **GraphQL introspection**: Smart query normalization
- **Compression**: Zstd for large bodies (optional)
- **Encryption**: AES-256-GCM for sensitive recordings
- **GUI**: Web-based recording browser

## Team Structure

**Required skills**:
- Rust systems programming
- Network protocols (HTTP, WebSocket, TLS)
- Performance optimization
- Security best practices

**Recommended team size**: 2-3 engineers for 12 weeks

## References

- [RFC Index](rfc/README.md)
- [TigerBeetle Style Guide](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Original test-server](https://github.com/google/test-server)

---

**Last Updated**: 2025-11-20  
**Status**: 🔵 Draft  
**Version**: 0.1.0

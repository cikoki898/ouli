# Ouli Roadmap to Production

## Status: 86% Complete (6/7 Milestones)

### Completed Milestones ✅
- [x] Milestone 1: Core Infrastructure
- [x] Milestone 2: Network Layer (stubs)
- [x] Milestone 3: Recording Engine
- [x] Milestone 4: Replay Engine
- [x] Milestone 5: Proxy Integration
- [x] Milestone 6: Testing & Validation (with known issues)

### In Progress 🚧
- [ ] Milestone 7: Production Readiness

## Outstanding Issues (Pre-Release)

### High Priority (Blocking Production) 🔴

#### #15: Implement Real HTTP Client Forwarding
**Status:** Open  
**Effort:** Large  
**Files:** `src/proxy/http.rs`, `src/network/http.rs`  
**Blockers:** None  
**Description:** Replace mock HTTP forwarding with real hyper client implementation.

**Why Critical:**
- Currently returns mock responses only
- No actual proxying happening
- Core functionality requirement

**Timeline:** Sprint 1 (1-2 weeks)

---

### Medium Priority (Important for Completeness) 🟡

#### #16: Implement WebSocket Proxying
**Status:** Open  
**Effort:** Large  
**Files:** `src/network/websocket.rs`, proxy modules  
**Blockers:** None (can be parallel with #15)  
**Description:** Full WebSocket handshake and frame-based recording/replay.

**Why Important:**
- Advertised feature
- Full network protocol support
- Common use case (real-time apps)

**Timeline:** Sprint 2 (1-2 weeks)

---

#### #19: Fix RequestChain State Sharing
**Status:** Open  
**Effort:** Medium  
**Files:** `src/storage/*`, `src/fingerprint.rs`, tests  
**Blockers:** None  
**Description:** Enable end-to-end record-replay testing by persisting chain state.

**Why Important:**
- 2 integration tests currently ignored
- Better test coverage
- Validates production workflow

**Timeline:** Sprint 2 (3-5 days)

---

### Low Priority (Optimizations) 🟢

#### #17: Optimize Recording Lookup
**Status:** Open  
**Effort:** Medium  
**Files:** `src/storage/reader.rs`, `src/storage/writer.rs`  
**Blockers:** None  
**Description:** Replace O(n) linear search with O(log n) binary search or O(1) hash table.

**Why Nice-to-Have:**
- Performance improvement
- Scales better with large recordings
- Not blocking for small/medium use cases

**Timeline:** Sprint 3 (2-3 days)

---

#### #18: Enhanced Binary Serialization
**Status:** Open  
**Effort:** Large  
**Files:** `src/recording/engine.rs`, `src/replay/cache.rs`  
**Blockers:** None  
**Description:** Add compression, schema versioning, and more efficient encoding.

**Why Nice-to-Have:**
- Current format works
- Space/speed optimizations
- Future-proofing

**Timeline:** Sprint 4+ (1 week)

---

## Milestone 7: Production Readiness

### Required Before Release

#### Documentation 📚
- [ ] Comprehensive README with examples
- [ ] API documentation (rustdoc)
- [ ] Configuration guide
- [ ] Recording format specification
- [ ] Troubleshooting guide
- [ ] Architecture diagram

#### CLI Interface 🖥️
- [ ] Command-line tool (`ouli` binary)
- [ ] Record mode command
- [ ] Replay mode command
- [ ] Config validation command
- [ ] Status/stats command

#### Observability 📊
- [ ] Structured logging (tracing)
- [ ] Metrics exports (Prometheus?)
- [ ] Performance counters
- [ ] Error tracking

#### Hardening 🛡️
- [ ] Input validation
- [ ] Rate limiting
- [ ] Resource limits enforcement
- [ ] Graceful shutdown
- [ ] Signal handling

#### Security 🔒
- [ ] Audit dependencies
- [ ] Security policy (SECURITY.md)
- [ ] Fuzzing tests
- [ ] Vulnerability scanning

## Proposed Release Schedule

### Sprint 1 (Week 1-2): Core Functionality
**Goal:** Real HTTP forwarding working

- [x] Create issues for all TODOs (#15-19)
- [ ] Implement HTTP client forwarding (#15)
- [ ] Integration tests with real endpoints
- [ ] Fix any discovered issues

**Deliverable:** Working record-replay for HTTP traffic

---

### Sprint 2 (Week 3-4): Feature Completion
**Goal:** Full protocol support + testing improvements

- [ ] Implement WebSocket proxying (#16)
- [ ] Fix RequestChain state sharing (#19)
- [ ] WebSocket integration tests
- [ ] Un-ignore test suite

**Deliverable:** Complete network protocol support

---

### Sprint 3 (Week 5): Optimization & CLI
**Goal:** Performance + usability

- [ ] Optimize recording lookup (#17)
- [ ] Build CLI interface
- [ ] Add observability (logging, metrics)
- [ ] Performance benchmarks validation

**Deliverable:** Production-grade performance and UX

---

### Sprint 4 (Week 6): Documentation & Hardening
**Goal:** Production readiness

- [ ] Complete documentation
- [ ] Security audit
- [ ] Hardening (limits, validation, etc.)
- [ ] Final integration testing
- [ ] Load testing

**Deliverable:** Release Candidate 1

---

### Sprint 5 (Week 7): Release
**Goal:** v1.0.0 🎉

- [ ] Address RC1 feedback
- [ ] Final documentation polish
- [ ] Release notes
- [ ] Tag v1.0.0
- [ ] Publish to crates.io

**Deliverable:** v1.0.0 Release

---

## Optional Enhancements (Post-1.0)

### Performance
- [ ] Enhanced serialization format (#18)
- [ ] Zero-copy replay (Arc<CachedResponse>)
- [ ] Parallel cache warming
- [ ] Streaming large responses

### Features
- [ ] Request/response transformation
- [ ] Replay with modifications
- [ ] Multiple recording merge
- [ ] Diff tool for recordings
- [ ] Web UI for replay management

### Integrations
- [ ] Docker image
- [ ] Kubernetes deployment
- [ ] CI/CD plugins (GitHub Actions, etc.)
- [ ] Language-specific clients

---

## Success Metrics

### Pre-Release
- [ ] All high-priority issues resolved
- [ ] Zero ignored tests
- [ ] < 100ms p99 latency for replay
- [ ] > 1000 req/s throughput
- [ ] Zero memory leaks in 24h test
- [ ] Zero data corruption in 1M requests

### Post-Release
- [ ] 90% test coverage
- [ ] < 10 reported bugs in first month
- [ ] Positive community feedback
- [ ] At least 3 production users

---

## Decision Log

### 2025-11-21: Identified Outstanding TODOs
**Decision:** Create GitHub issues for all TODOs before proceeding with Milestone 7  
**Rationale:** Better tracking, prioritization, and planning  
**Issues Created:** #15, #16, #17, #18, #19

---

*Last Updated: 2025-11-21*

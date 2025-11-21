# Ouli RFCs

Ouli is a deterministic HTTP/WebSocket record-replay proxy built in Rust with TigerBeetle principles.

## Design Goals

1. **Safety**: Memory-safe handling of untrusted network input
2. **Performance**: Zero-copy I/O, memory-mapped storage, < 100μs replay latency
3. **Determinism**: Identical replays guaranteed by design
4. **Simplicity**: Explicit control flow, bounded resources, no technical debt

## RFC Index

- [RFC-001: Architecture Overview](001-architecture.md)
- [RFC-002: Binary Storage Format](002-binary-format.md)
- [RFC-003: Request Fingerprinting](003-request-fingerprinting.md)
- [RFC-004: Network Protocol Handler](004-network-handler.md)
- [RFC-005: Recording Engine](005-recording-engine.md)
- [RFC-006: Replay Engine](006-replay-engine.md)
- [RFC-007: Security and Redaction](007-security-redaction.md)
- [RFC-008: Performance Optimization](008-performance.md)
- [RFC-009: Testing Strategy](009-testing.md)
- [RFC-010: SDK Design](010-sdk-design.md)

## Status Legend

- 🔵 **Draft**: Under discussion
- 🟡 **Accepted**: Ready for implementation
- 🟢 **Implemented**: Code complete
- 🔴 **Deprecated**: Superseded by newer RFC

## Principles

Every RFC must:

- Put a limit on everything
- Assert all invariants
- Handle all errors explicitly
- Prefer compile-time guarantees over runtime checks
- Use simple, explicit control flow
- Maintain zero technical debt

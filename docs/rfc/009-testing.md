# RFC-009: Testing Strategy

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Define a comprehensive testing strategy ensuring correctness, safety, and performance through unit tests, integration tests, property-based testing, fuzzing, and chaos engineering.

## Testing Pyramid

```
                  ┌─────────────┐
                  │   Manual    │ < 1%
                  └─────────────┘
              ┌─────────────────────┐
              │   E2E / Chaos       │ 5%
              └─────────────────────┘
          ┌───────────────────────────────┐
          │   Integration / Fuzzing       │ 20%
          └───────────────────────────────┘
      ┌─────────────────────────────────────────┐
      │   Unit / Property Tests                 │ 75%
      └─────────────────────────────────────────┘
```

## Unit Tests

### Coverage Requirements

- **Safety-critical code**: 100% coverage
- **Core logic**: > 95% coverage
- **Utilities**: > 90% coverage
- **Generated code**: Exempt

```rust
// Example: Request fingerprinting
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fingerprint_deterministic() {
        let req = test_request();
        let hash1 = fingerprint_request(&req, CHAIN_HEAD_HASH, &Redactor::default());
        let hash2 = fingerprint_request(&req, CHAIN_HEAD_HASH, &Redactor::default());
        
        assert_eq!(hash1, hash2, "Fingerprint must be deterministic");
    }
    
    #[test]
    fn test_fingerprint_sensitivity() {
        let req1 = Request::get("/api").body("data1");
        let req2 = Request::get("/api").body("data2");
        
        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH, &Redactor::default());
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH, &Redactor::default());
        
        assert_ne!(hash1, hash2, "Different requests must have different hashes");
    }
    
    #[test]
    fn test_fingerprint_normalization() {
        // Headers in different order
        let req1 = Request::get("/api")
            .header("a", "1")
            .header("b", "2")
            .build();
        
        let req2 = Request::get("/api")
            .header("b", "2")
            .header("a", "1")
            .build();
        
        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH, &Redactor::default());
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH, &Redactor::default());
        
        assert_eq!(hash1, hash2, "Header order should not affect fingerprint");
    }
}
```

## Property-Based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_fingerprint_collision_resistance(
        seed in any::<u64>(),
        iterations in 100..1000usize,
    ) {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut seen = HashSet::new();
        
        for _ in 0..iterations {
            let request = random_request(&mut rng);
            let hash = fingerprint_request(&request, CHAIN_HEAD_HASH, &Redactor::default());
            
            prop_assert!(
                seen.insert(hash),
                "Hash collision detected"
            );
        }
    }
    
    #[test]
    fn prop_redaction_completeness(
        secret in "[a-zA-Z0-9]{16,64}",
        text in ".*",
    ) {
        let redactor = Redactor::new(&RedactionConfig {
            secrets: vec![secret.clone()],
            ..Default::default()
        }).unwrap();
        
        let redacted = redactor.redact_str(&text);
        
        prop_assert!(
            !redacted.contains(&secret),
            "Secret leaked after redaction: {}",
            secret
        );
    }
    
    #[test]
    fn prop_recording_roundtrip(
        interactions in prop::collection::vec(
            (arbitrary_request(), arbitrary_response()),
            1..100
        )
    ) {
        let path = temp_file();
        let recording_id = [0u8; 32];
        
        // Write
        {
            let mut writer = RecordingWriter::create(&path, recording_id).unwrap();
            
            for (req, resp) in &interactions {
                writer.append_interaction(
                    compute_hash(req),
                    CHAIN_HEAD_HASH,
                    req,
                    resp,
                ).unwrap();
            }
            
            writer.finalize().unwrap();
        }
        
        // Read back
        {
            let reader = RecordingReader::open(&path).unwrap();
            
            prop_assert_eq!(
                reader.interaction_count(),
                interactions.len(),
                "Interaction count mismatch"
            );
            
            for (req, _) in &interactions {
                let hash = compute_hash(req);
                prop_assert!(
                    reader.lookup(hash).is_some(),
                    "Interaction not found: {:?}",
                    hash
                );
            }
        }
    }
}
```

## Fuzzing

### libFuzzer Integration

```rust
// fuzz/fuzz_targets/fingerprint.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
use ouli::fingerprint::fingerprint_request;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    
    if let Ok(request) = parse_http_request(data) {
        let _ = fingerprint_request(&request, CHAIN_HEAD_HASH, &Redactor::default());
    }
});

// fuzz/fuzz_targets/binary_format.rs
fuzz_target!(|data: &[u8]| {
    if data.len() < 128 {
        return;
    }
    
    // Should never panic, only return error
    let _ = RecordingReader::from_bytes(data);
});

// fuzz/fuzz_targets/redaction.rs
fuzz_target!(|data: (Vec<String>, String)| {
    let (secrets, text) = data;
    
    if secrets.is_empty() || secrets.iter().any(|s| s.is_empty()) {
        return;
    }
    
    let config = RedactionConfig {
        secrets,
        ..Default::default()
    };
    
    if let Ok(redactor) = Redactor::new(&config) {
        let redacted = redactor.redact_str(&text);
        
        // Verify no secrets leaked
        for secret in &config.secrets {
            assert!(!redacted.contains(secret), "Secret leaked: {}", secret);
        }
    }
});
```

### Continuous Fuzzing

```yaml
# .github/workflows/fuzz.yml
name: Continuous Fuzzing

on:
  schedule:
    - cron: '0 */6 * * *'  # Every 6 hours

jobs:
  fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: nightly
          
      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz
        
      - name: Run fuzzers
        run: |
          for target in fuzz/fuzz_targets/*.rs; do
            name=$(basename $target .rs)
            timeout 5m cargo +nightly fuzz run $name || true
          done
          
      - name: Upload artifacts
        if: failure()
        uses: actions/upload-artifact@v3
        with:
          name: fuzz-artifacts
          path: fuzz/artifacts/
```

## Integration Tests

```rust
#[tokio::test]
async fn test_record_and_replay_flow() {
    let temp_dir = tempdir().unwrap();
    
    // Start mock target server
    let target = MockServer::start().await;
    target.mock_get("/api/test").with_status(200).with_body("OK").create().await;
    
    // Configure Ouli
    let config = Config {
        mode: Mode::Record,
        recording_dir: temp_dir.path().to_path_buf(),
        endpoints: vec![EndpointConfig {
            target_host: target.host(),
            target_port: target.port(),
            source_port: 8080,
            ..Default::default()
        }],
        ..Default::default()
    };
    
    // Start recording proxy
    let proxy = start_proxy(config.clone()).await.unwrap();
    
    // Make request through proxy
    let client = reqwest::Client::new();
    let response = client.get("http://localhost:8080/api/test")
        .header("x-ouli-test-name", "integration_test")
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "OK");
    
    // Stop proxy and finalize recording
    proxy.shutdown().await.unwrap();
    
    // Verify recording file exists
    let recording_path = temp_dir.path().join("integration_test.ouli");
    assert!(recording_path.exists());
    
    // Switch to replay mode
    let mut config = config;
    config.mode = Mode::Replay;
    
    let proxy = start_proxy(config).await.unwrap();
    
    // Make same request (target server can be down now)
    target.shutdown().await;
    
    let response = client.get("http://localhost:8080/api/test")
        .header("x-ouli-test-name", "integration_test")
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "OK");
    
    proxy.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_websocket_record_replay() {
    let temp_dir = tempdir().unwrap();
    
    // Similar to above but for WebSocket
    let target = MockWebSocketServer::start().await;
    
    // ... record websocket interaction ...
    
    // ... replay websocket interaction ...
}

#[tokio::test]
async fn test_concurrent_sessions() {
    let proxy = start_test_proxy().await;
    
    let mut tasks = vec![];
    
    for i in 0..100 {
        tasks.push(tokio::spawn(async move {
            let client = reqwest::Client::new();
            client.get(format!("http://localhost:8080/test/{}", i))
                .header("x-ouli-test-name", format!("session_{}", i))
                .send()
                .await
                .unwrap()
        }));
    }
    
    for task in tasks {
        let response = task.await.unwrap();
        assert_eq!(response.status(), 200);
    }
}
```

## Chaos Testing

```rust
#[tokio::test]
#[ignore] // Run separately
async fn chaos_test_random_failures() {
    let proxy = start_test_proxy().await;
    
    let mut rng = rand::thread_rng();
    let duration = Duration::from_secs(60);
    let start = Instant::now();
    
    while start.elapsed() < duration {
        // Random actions
        match rng.gen_range(0..10) {
            0..=5 => {
                // Normal request
                let _ = make_request().await;
            }
            6 => {
                // Kill random connection
                proxy.kill_random_connection().await;
            }
            7 => {
                // Pause proxy briefly
                proxy.pause(Duration::from_millis(100)).await;
            }
            8 => {
                // Corrupt random file
                corrupt_random_recording(&proxy.recording_dir()).await;
            }
            9 => {
                // Restart proxy
                proxy.restart().await;
            }
            _ => unreachable!(),
        }
        
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Verify proxy still functional
    let response = make_request().await.unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
#[ignore]
async fn chaos_test_resource_exhaustion() {
    let proxy = start_test_proxy().await;
    
    // Try to exhaust connections
    let mut connections = vec![];
    
    for _ in 0..10_000 {
        match tokio::spawn(make_request()).await {
            Ok(_) => connections.push(()),
            Err(_) => break,
        }
    }
    
    // Should hit limit gracefully
    assert!(connections.len() <= MAX_CONNECTIONS);
    
    // Should recover after connections close
    drop(connections);
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    let response = make_request().await.unwrap();
    assert_eq!(response.status(), 200);
}
```

## Performance Regression Tests

```rust
#[test]
fn bench_regression_fingerprint() {
    let baseline = Duration::from_micros(10);
    
    let request = test_request();
    let redactor = Redactor::default();
    
    let start = Instant::now();
    for _ in 0..1000 {
        fingerprint_request(&request, CHAIN_HEAD_HASH, &redactor);
    }
    let avg = start.elapsed() / 1000;
    
    assert!(
        avg < baseline,
        "Fingerprint regression: {}μs > {}μs",
        avg.as_micros(),
        baseline.as_micros()
    );
}

#[tokio::test]
async fn bench_regression_replay_latency() {
    let engine = setup_replay_engine().await;
    
    let mut latencies = vec![];
    
    for _ in 0..10_000 {
        let start = Instant::now();
        let _ = engine.replay_interaction(test_request(), &test_endpoint()).await.unwrap();
        latencies.push(start.elapsed());
    }
    
    latencies.sort_unstable();
    
    let p99 = latencies[latencies.len() * 99 / 100];
    
    assert!(
        p99 < Duration::from_micros(100),
        "p99 regression: {:?} > 100μs",
        p99
    );
}
```

## Test Helpers

```rust
pub mod helpers {
    pub fn test_request() -> Request<Full<Bytes>> {
        Request::builder()
            .method("GET")
            .uri("/api/test")
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from("{}")))
            .unwrap()
    }
    
    pub fn random_request(rng: &mut impl Rng) -> Request<Full<Bytes>> {
        let methods = ["GET", "POST", "PUT", "DELETE"];
        let paths = ["/api/v1", "/api/v2", "/health", "/metrics"];
        
        Request::builder()
            .method(methods[rng.gen_range(0..methods.len())])
            .uri(paths[rng.gen_range(0..paths.len())])
            .body(Full::new(Bytes::from(random_json(rng))))
            .unwrap()
    }
    
    pub fn arbitrary_request() -> impl Strategy<Value = RequestData> {
        (
            "[A-Z]{3,7}",  // Method
            "/[a-z/]{1,50}",  // Path
            prop::collection::vec(
                ("[a-z-]{3,20}", "[a-z0-9]{3,50}"),
                0..20
            ),  // Headers
            prop::collection::vec(any::<u8>(), 0..1000)  // Body
        ).prop_map(|(method, path, headers, body)| {
            RequestData { method, path, headers, body }
        })
    }
}
```

## CI/CD Integration

```yaml
# .github/workflows/test.yml
name: Test Suite

on: [push, pull_request]

jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        rust: [stable, nightly]
    
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: ${{ matrix.rust }}
          
      - name: Run unit tests
        run: cargo test --all-features
        
      - name: Run integration tests
        run: cargo test --test '*' --all-features
        
      - name: Run property tests
        run: cargo test --release -- --include-ignored proptest
        
      - name: Generate coverage
        if: matrix.os == 'ubuntu-latest' && matrix.rust == 'stable'
        run: |
          cargo install cargo-tarpaulin
          cargo tarpaulin --out Xml
          
      - name: Upload coverage
        if: matrix.os == 'ubuntu-latest' && matrix.rust == 'stable'
        uses: codecov/codecov-action@v3
```

## Test Metrics

Track test health:

```rust
pub struct TestMetrics {
    pub total_tests: usize,
    pub passing: usize,
    pub failing: usize,
    pub coverage_percent: f64,
    pub avg_runtime_ms: f64,
}
```

## References

- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [Proptest Book](https://altsysrq.github.io/proptest-book/)
- [Chaos Engineering](https://principlesofchaos.org/)
- [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html)

# RFC-003: Request Fingerprinting

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Define a deterministic cryptographic fingerprinting algorithm for HTTP requests that enables content-addressable storage and guaranteed replay matching.

## Motivation

**Problem**: HTTP requests must map to stored responses deterministically.

**Challenges**:
- Header order varies between clients
- Whitespace in headers inconsistent
- Case-sensitivity (HTTP headers are case-insensitive)
- Query parameter ordering
- Body encoding variations (JSON key order)

**Requirements**:
- Same logical request → same fingerprint
- Different requests → different fingerprints (collision-resistant)
- Chain-aware (sequential requests linked)
- Redaction-compatible (secrets removed before hashing)

## Algorithm

### Core Fingerprint

```rust
pub fn fingerprint_request(
    request: &Request,
    prev_hash: [u8; 32],
    redactor: &Redactor,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    
    // 1. Method (uppercase normalized)
    let method = request.method().as_str().to_uppercase();
    hasher.update(&(method.len() as u32).to_le_bytes());
    hasher.update(method.as_bytes());
    
    // 2. Path (decoded, normalized)
    let path = normalize_path(request.uri().path());
    hasher.update(&(path.len() as u32).to_le_bytes());
    hasher.update(path.as_bytes());
    
    // 3. Query parameters (sorted)
    if let Some(query) = request.uri().query() {
        let params = parse_and_sort_query(query);
        for (key, value) in params {
            hasher.update(&(key.len() as u32).to_le_bytes());
            hasher.update(key.as_bytes());
            hasher.update(&(value.len() as u32).to_le_bytes());
            hasher.update(value.as_bytes());
        }
    }
    
    // 4. Headers (sorted, normalized, redacted)
    let headers = normalize_headers(request.headers(), redactor);
    for (name, value) in headers {
        hasher.update(&(name.len() as u32).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&(value.len() as u32).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    
    // 5. Body (normalized, redacted)
    let body = normalize_body(request.body(), request.headers(), redactor);
    hasher.update(&(body.len() as u32).to_le_bytes());
    hasher.update(&body);
    
    // 6. Previous request hash (chain linkage)
    hasher.update(&prev_hash);
    
    hasher.finalize().into()
}
```

### Normalization Rules

#### Path Normalization

```rust
fn normalize_path(path: &str) -> String {
    assert!(path.len() <= MAX_PATH_LEN);
    
    // 1. Decode percent-encoding
    let decoded = percent_decode_str(path).decode_utf8_lossy();
    
    // 2. Collapse repeated slashes
    let collapsed = decoded.split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    
    // 3. Ensure leading slash
    format!("/{}", collapsed)
}

#[test]
fn test_path_normalization() {
    assert_eq!(normalize_path("/api/v1"), "/api/v1");
    assert_eq!(normalize_path("/api//v1"), "/api/v1");
    assert_eq!(normalize_path("/api/%2F/v1"), "/api///v1");
    assert_eq!(normalize_path("api/v1"), "/api/v1");
}
```

#### Query Parameter Sorting

```rust
fn parse_and_sort_query(query: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            let decoded_key = percent_decode_str(key).decode_utf8_lossy();
            let decoded_value = percent_decode_str(value).decode_utf8_lossy();
            params.insert(decoded_key.into_owned(), decoded_value.into_owned());
        }
    }
    
    params
}

#[test]
fn test_query_sorting() {
    let query1 = "b=2&a=1&c=3";
    let query2 = "a=1&c=3&b=2";
    
    assert_eq!(
        parse_and_sort_query(query1),
        parse_and_sort_query(query2)
    );
}
```

#### Header Normalization

```rust
fn normalize_headers(
    headers: &HeaderMap,
    redactor: &Redactor,
) -> BTreeMap<String, String> {
    let mut normalized = BTreeMap::new();
    
    for (name, value) in headers {
        // Skip headers that shouldn't affect matching
        if is_excluded_header(name) {
            continue;
        }
        
        let name_lower = name.as_str().to_lowercase();
        let value_str = value.to_str().unwrap_or("");
        
        // Redact sensitive values
        let redacted = redactor.redact_str(value_str);
        
        // Trim whitespace
        let trimmed = redacted.trim();
        
        normalized.insert(name_lower, trimmed.to_string());
    }
    
    normalized
}

fn is_excluded_header(name: &HeaderName) -> bool {
    const EXCLUDED: &[&str] = &[
        "date",
        "age",
        "expires",
        "connection",
        "keep-alive",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];
    
    EXCLUDED.contains(&name.as_str().to_lowercase().as_str())
}
```

#### Body Normalization

```rust
fn normalize_body(
    body: &[u8],
    headers: &HeaderMap,
    redactor: &Redactor,
) -> Vec<u8> {
    assert!(body.len() <= MAX_BODY_SIZE);
    
    // Detect content type
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    if content_type.contains("application/json") {
        normalize_json_body(body, redactor)
    } else if content_type.contains("application/x-www-form-urlencoded") {
        normalize_form_body(body, redactor)
    } else {
        // Binary/text: redact and return
        redactor.redact_bytes(body)
    }
}

fn normalize_json_body(body: &[u8], redactor: &Redactor) -> Vec<u8> {
    // Parse JSON
    let value: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return redactor.redact_bytes(body),
    };
    
    // Redact sensitive fields
    let redacted = redact_json_value(value, redactor);
    
    // Serialize with sorted keys (deterministic)
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"");
    let mut serializer = serde_json::Serializer::with_formatter(&mut buf, formatter);
    
    redacted.serialize(&mut serializer).unwrap();
    
    buf
}

fn redact_json_value(
    value: serde_json::Value,
    redactor: &Redactor,
) -> serde_json::Value {
    use serde_json::Value;
    
    match value {
        Value::String(s) => {
            Value::String(redactor.redact_str(&s).to_string())
        }
        Value::Object(map) => {
            // BTreeMap ensures sorted iteration
            let mut result = serde_json::Map::new();
            for (k, v) in map {
                result.insert(k, redact_json_value(v, redactor));
            }
            Value::Object(result)
        }
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(|v| redact_json_value(v, redactor)).collect())
        }
        other => other,
    }
}

#[test]
fn test_json_normalization() {
    let redactor = Redactor::new(&[]);
    
    let json1 = br#"{"b": 2, "a": 1}"#;
    let json2 = br#"{"a": 1, "b": 2}"#;
    let json3 = br#"{"a":1,"b":2}"#; // No spaces
    
    let norm1 = normalize_json_body(json1, &redactor);
    let norm2 = normalize_json_body(json2, &redactor);
    let norm3 = normalize_json_body(json3, &redactor);
    
    assert_eq!(norm1, norm2);
    assert_eq!(norm2, norm3);
}
```

## Chain Linking

Each request hash includes the previous request hash, creating a deterministic chain:

```rust
pub struct RequestChain {
    current_hash: [u8; 32],
}

impl RequestChain {
    pub fn new() -> Self {
        Self {
            current_hash: CHAIN_HEAD_HASH,
        }
    }
    
    pub fn process_request(&mut self, request: &Request, redactor: &Redactor) -> [u8; 32] {
        let hash = fingerprint_request(request, self.current_hash, redactor);
        self.current_hash = hash;
        hash
    }
    
    pub fn reset(&mut self) {
        self.current_hash = CHAIN_HEAD_HASH;
    }
}

/// Special hash for chain head (first request)
pub const CHAIN_HEAD_HASH: [u8; 32] = [
    0xb4, 0xd6, 0xe6, 0x0a, 0x9b, 0x97, 0xe7, 0xb9,
    0x8c, 0x63, 0xdf, 0x93, 0x08, 0x72, 0x8c, 0x5c,
    0x88, 0xc0, 0xb4, 0x0c, 0x39, 0x80, 0x46, 0x77,
    0x2c, 0x63, 0x44, 0x7b, 0x94, 0x60, 0x8b, 0x4d,
];
```

### Chain Reset

Chains reset on:
1. New test/recording file
2. Explicit reset via header: `X-Ouli-Reset-Chain: true`
3. Connection close

```rust
fn should_reset_chain(request: &Request) -> bool {
    request.headers()
        .get("x-ouli-reset-chain")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
```

## Custom Test Names

Override hash-based naming with header:

```rust
fn get_recording_name(request: &Request, hash: [u8; 32]) -> String {
    if let Some(name) = request.headers()
        .get("x-ouli-test-name")
        .and_then(|v| v.to_str().ok())
    {
        validate_test_name(name).unwrap_or_else(|_| hex_hash(hash))
    } else {
        hex_hash(hash)
    }
}

fn validate_test_name(name: &str) -> Result<String, ValidationError> {
    // No path traversal
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(ValidationError::InvalidCharacters);
    }
    
    // Reasonable length
    if name.is_empty() || name.len() > 255 {
        return Err(ValidationError::InvalidLength);
    }
    
    // Safe characters only
    if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        return Err(ValidationError::InvalidCharacters);
    }
    
    Ok(name.to_string())
}

#[test]
fn test_name_validation() {
    assert!(validate_test_name("test_login").is_ok());
    assert!(validate_test_name("test-api-v2").is_ok());
    assert!(validate_test_name("../etc/passwd").is_err());
    assert!(validate_test_name("test/name").is_err());
    assert!(validate_test_name("").is_err());
}
```

## Collision Resistance

SHA-256 provides:
- 2^256 possible hashes
- 2^128 operations for 50% collision probability (birthday paradox)

**Practical limit**: ~10^38 unique requests before collision concern.

**Mitigation**: If collision detected, append sequence number:

```rust
fn handle_collision(
    base_hash: [u8; 32],
    existing_request: &Request,
    new_request: &Request,
) -> [u8; 32] {
    // Verify actual collision (not just hash reuse)
    if requests_equal(existing_request, new_request) {
        return base_hash; // Same request, same hash
    }
    
    // True collision: append counter
    warn!("Hash collision detected: {:x}", base_hash);
    
    let mut hasher = Sha256::new();
    hasher.update(&base_hash);
    hasher.update(&1u32.to_le_bytes()); // Counter
    hasher.finalize().into()
}
```

## Performance

### Benchmarks

Target performance on modern CPU:

| Operation | Target | Notes |
|-----------|--------|-------|
| Hash computation | < 10 μs | For typical request |
| Path normalization | < 1 μs | Simple string ops |
| Header normalization | < 5 μs | 20 headers |
| JSON normalization | < 20 μs | 1KB JSON |
| Query sort | < 2 μs | 10 parameters |

### Optimization: Incremental Hashing

For large bodies, use incremental hashing:

```rust
fn hash_large_body(body: &[u8]) -> [u8; 32] {
    const CHUNK_SIZE: usize = 64 * 1024; // 64 KB
    
    assert!(body.len() <= MAX_BODY_SIZE);
    
    let mut hasher = Sha256::new();
    
    for chunk in body.chunks(CHUNK_SIZE) {
        hasher.update(chunk);
    }
    
    hasher.finalize().into()
}
```

## Testing

### Property Tests

```rust
#[cfg(test)]
mod proptests {
    use proptest::prelude::*;
    
    proptest! {
        #[test]
        fn same_request_same_hash(
            method in "GET|POST|PUT",
            path in "/[a-z]{1,10}",
            body in prop::collection::vec(any::<u8>(), 0..1000),
        ) {
            let req1 = build_request(&method, &path, &body);
            let req2 = build_request(&method, &path, &body);
            
            let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH, &Redactor::new(&[]));
            let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH, &Redactor::new(&[]));
            
            prop_assert_eq!(hash1, hash2);
        }
        
        #[test]
        fn different_request_different_hash(
            method1 in "GET|POST",
            method2 in "PUT|DELETE",
            path in "/[a-z]{1,10}",
        ) {
            let req1 = build_request(&method1, &path, &[]);
            let req2 = build_request(&method2, &path, &[]);
            
            let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH, &Redactor::new(&[]));
            let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH, &Redactor::new(&[]));
            
            prop_assert_ne!(hash1, hash2);
        }
    }
}
```

### Determinism Tests

```rust
#[test]
fn determinism_across_runs() {
    let request = build_test_request();
    let redactor = Redactor::new(&[]);
    
    let hashes: Vec<[u8; 32]> = (0..1000)
        .map(|_| fingerprint_request(&request, CHAIN_HEAD_HASH, &redactor))
        .collect();
    
    // All hashes must be identical
    assert!(hashes.windows(2).all(|w| w[0] == w[1]));
}
```

## Edge Cases

### Empty Body

```rust
assert_eq!(
    fingerprint_request(&Request::get("/api").body("").build(), ...),
    fingerprint_request(&Request::get("/api").build(), ...)
);
```

### Case Sensitivity

```rust
// Headers are case-insensitive
let req1 = Request::get("/").header("Content-Type", "application/json");
let req2 = Request::get("/").header("content-type", "application/json");

assert_eq!(fingerprint_request(&req1, ...), fingerprint_request(&req2, ...));
```

### Unicode

```rust
// Properly handle UTF-8
let req = Request::get("/api/用户").build();
let hash = fingerprint_request(&req, CHAIN_HEAD_HASH, &Redactor::new(&[]));
assert!(hash != [0; 32]);
```

## Open Questions

1. **Streaming bodies**: Hash as they stream or buffer?
   - **Proposal**: Buffer up to limit, reject oversized
   
2. **Binary formats**: Should we parse Protobuf/MessagePack?
   - **Proposal**: Treat as opaque bytes for v1.0

3. **GraphQL**: Special handling for query normalization?
   - **Proposal**: Treat as JSON for v1.0

## References

- [SHA-256 Specification](https://nvlpubs.nist.gov/nistpubs/FIPS/NIST.FIPS.180-4.pdf)
- [HTTP Header Normalization](https://www.rfc-editor.org/rfc/rfc7230)
- [JSON Canonicalization](https://www.rfc-editor.org/rfc/rfc8785)
- [Percent Encoding](https://www.rfc-editor.org/rfc/rfc3986)

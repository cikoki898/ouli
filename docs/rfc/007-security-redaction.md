# RFC-007: Security and Redaction

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Define comprehensive security measures for handling sensitive data, including redaction of secrets, TLS configuration, and protection against common attacks.

## Threat Model

**Assets**:
- API keys, tokens, passwords in headers/bodies
- PII in request/response data
- Internal infrastructure details
- Recording files on disk

**Threats**:
- T1: Secret leakage in recordings
- T2: Path traversal in custom test names
- T3: Denial of service via resource exhaustion
- T4: Man-in-the-middle attacks on target connections
- T5: Unauthorized access to recordings
- T6: Replay attacks (if recordings used maliciously)

## Redaction Engine

```rust
pub struct Redactor {
    patterns: Vec<CompiledPattern>,
    header_keys: HashSet<HeaderName>,
    json_paths: Vec<JsonPath>,
}

pub struct CompiledPattern {
    needle: Bytes,
    replacement: Bytes,
    skip_table: [usize; 256], // Boyer-Moore optimization
}

impl Redactor {
    pub fn new(config: &RedactionConfig) -> Result<Self> {
        let mut patterns = Vec::new();
        
        // Compile literal secrets
        for secret in &config.secrets {
            assert!(!secret.is_empty());
            assert!(secret.len() <= MAX_SECRET_LEN);
            
            patterns.push(Self::compile_pattern(
                secret.as_bytes(),
                b"REDACTED",
            ));
        }
        
        // Compile regex patterns
        for pattern in &config.regex_patterns {
            // Pre-compile for performance
            let regex = Regex::new(pattern)?;
            patterns.push(Self::compile_regex_pattern(regex));
        }
        
        Ok(Self {
            patterns,
            header_keys: config.redact_headers.iter().cloned().collect(),
            json_paths: config.json_paths.clone(),
        })
    }
}
```

## Header Redaction

```rust
impl Redactor {
    pub fn redact_headers(&self, headers: &mut HeaderMap) {
        // Remove sensitive headers entirely
        for key in &self.header_keys {
            headers.remove(key);
        }
        
        // Redact values in remaining headers
        for (_name, value) in headers.iter_mut() {
            if let Ok(v) = value.to_str() {
                let redacted = self.redact_str(v);
                *value = HeaderValue::from_str(&redacted).unwrap();
            }
        }
    }
}
```

## Body Redaction

### String Redaction (Boyer-Moore)

```rust
impl Redactor {
    pub fn redact_str(&self, input: &str) -> String {
        if self.patterns.is_empty() {
            return input.to_string();
        }
        
        let mut output = input.to_string();
        
        for pattern in &self.patterns {
            output = self.boyer_moore_replace(&output, pattern);
        }
        
        output
    }
    
    fn boyer_moore_replace(&self, text: &str, pattern: &CompiledPattern) -> String {
        let text_bytes = text.as_bytes();
        let needle = &pattern.needle;
        let replacement = &pattern.replacement;
        
        if needle.is_empty() || text_bytes.len() < needle.len() {
            return text.to_string();
        }
        
        let mut result = Vec::new();
        let mut i = 0;
        
        while i <= text_bytes.len() - needle.len() {
            let mut j = needle.len();
            
            // Match from right to left
            while j > 0 && text_bytes[i + j - 1] == needle[j - 1] {
                j -= 1;
            }
            
            if j == 0 {
                // Match found
                result.extend_from_slice(replacement);
                i += needle.len();
            } else {
                // No match, use skip table
                result.push(text_bytes[i]);
                let skip = pattern.skip_table[text_bytes[i + needle.len() - 1] as usize];
                i += skip.max(1);
            }
        }
        
        // Append remaining
        result.extend_from_slice(&text_bytes[i..]);
        
        String::from_utf8(result).unwrap()
    }
    
    fn compile_pattern(needle: &[u8], replacement: &[u8]) -> CompiledPattern {
        // Build Boyer-Moore skip table
        let mut skip_table = [needle.len(); 256];
        
        for (i, &byte) in needle.iter().enumerate().take(needle.len() - 1) {
            skip_table[byte as usize] = needle.len() - 1 - i;
        }
        
        CompiledPattern {
            needle: Bytes::copy_from_slice(needle),
            replacement: Bytes::copy_from_slice(replacement),
            skip_table,
        }
    }
}
```

### JSON Redaction

```rust
impl Redactor {
    pub fn redact_json(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(s) => {
                *s = self.redact_str(s);
            }
            serde_json::Value::Object(map) => {
                for (key, val) in map.iter_mut() {
                    // Check if key matches sensitive path
                    if self.is_sensitive_key(key) {
                        *val = serde_json::Value::String("REDACTED".to_string());
                    } else {
                        self.redact_json(val);
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                for val in arr.iter_mut() {
                    self.redact_json(val);
                }
            }
            _ => {}
        }
    }
    
    fn is_sensitive_key(&self, key: &str) -> bool {
        const SENSITIVE_KEYS: &[&str] = &[
            "password",
            "secret",
            "token",
            "api_key",
            "apikey",
            "authorization",
            "auth",
            "credential",
            "private_key",
            "access_token",
            "refresh_token",
        ];
        
        let key_lower = key.to_lowercase();
        SENSITIVE_KEYS.iter().any(|&k| key_lower.contains(k))
    }
}
```

## Path Traversal Protection

```rust
pub fn validate_test_name(name: &str) -> Result<()> {
    // Length check
    if name.is_empty() {
        return Err(SecurityError::EmptyTestName);
    }
    
    if name.len() > 255 {
        return Err(SecurityError::TestNameTooLong);
    }
    
    // No path traversal
    if name.contains("..") {
        return Err(SecurityError::PathTraversal);
    }
    
    // No path separators
    if name.contains('/') || name.contains('\\') {
        return Err(SecurityError::InvalidCharacter);
    }
    
    // No null bytes
    if name.contains('\0') {
        return Err(SecurityError::NullByte);
    }
    
    // Only safe characters
    for c in name.chars() {
        if !c.is_alphanumeric() && c != '_' && c != '-' && c != '.' {
            return Err(SecurityError::InvalidCharacter);
        }
    }
    
    // No leading/trailing dots (hidden files)
    if name.starts_with('.') || name.ends_with('.') {
        return Err(SecurityError::HiddenFile);
    }
    
    Ok(())
}

#[test]
fn test_path_traversal_prevention() {
    assert!(validate_test_name("../etc/passwd").is_err());
    assert!(validate_test_name("..\\windows\\system32").is_err());
    assert!(validate_test_name("test/../secret").is_err());
    assert!(validate_test_name("/etc/passwd").is_err());
    assert!(validate_test_name("test\0name").is_err());
    assert!(validate_test_name(".hidden").is_err());
    
    assert!(validate_test_name("valid_test_name").is_ok());
    assert!(validate_test_name("test-123").is_ok());
}
```

## Rate Limiting

```rust
pub struct RateLimiter {
    limiters: DashMap<IpAddr, TokenBucket>,
    config: RateLimitConfig,
}

pub struct RateLimitConfig {
    pub requests_per_second: u32,
    pub burst_size: u32,
    pub ban_after_violations: u32,
    pub ban_duration: Duration,
}

impl RateLimiter {
    pub async fn check_rate_limit(&self, addr: IpAddr) -> Result<()> {
        let mut bucket = self.limiters
            .entry(addr)
            .or_insert_with(|| TokenBucket::new(self.config.burst_size));
        
        if !bucket.try_consume() {
            bucket.violations += 1;
            
            if bucket.violations >= self.config.ban_after_violations {
                bucket.banned_until = Some(Instant::now() + self.config.ban_duration);
                return Err(SecurityError::RateLimitExceeded);
            }
            
            return Err(SecurityError::TooManyRequests);
        }
        
        Ok(())
    }
}

pub struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
    violations: u32,
    banned_until: Option<Instant>,
}

impl TokenBucket {
    fn try_consume(&mut self) -> bool {
        // Check if banned
        if let Some(until) = self.banned_until {
            if Instant::now() < until {
                return false;
            }
            // Unban
            self.banned_until = None;
            self.violations = 0;
        }
        
        // Refill tokens
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * RATE).min(BURST as f64);
        self.last_refill = now;
        
        // Try to consume
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.violations = 0;
            true
        } else {
            false
        }
    }
}
```

## TLS Configuration

```rust
pub struct TlsConfig {
    pub client_config: Arc<ClientConfig>,
    pub verify_certificates: bool,
    pub allowed_ciphers: Vec<CipherSuite>,
}

impl TlsConfig {
    pub fn new_secure() -> Result<Self> {
        let mut root_store = RootCertStore::empty();
        root_store.add_trust_anchors(
            webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| {
                OwnedTrustAnchor::from_subject_spki_name_constraints(
                    ta.subject,
                    ta.spki,
                    ta.name_constraints,
                )
            })
        );
        
        let config = ClientConfig::builder()
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_safe_default_protocol_versions()?
            .with_root_certificates(root_store)
            .with_no_client_auth();
        
        Ok(Self {
            client_config: Arc::new(config),
            verify_certificates: true,
            allowed_ciphers: vec![
                CipherSuite::TLS13_AES_256_GCM_SHA384,
                CipherSuite::TLS13_AES_128_GCM_SHA256,
                CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
            ],
        })
    }
    
    pub fn new_insecure_for_testing() -> Result<Self> {
        warn!("Using insecure TLS config - DO NOT USE IN PRODUCTION");
        
        let mut config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(RootCertStore::empty())
            .with_no_client_auth();
        
        // Disable certificate verification (testing only!)
        config.dangerous()
            .set_certificate_verifier(Arc::new(NoVerifier));
        
        Ok(Self {
            client_config: Arc::new(config),
            verify_certificates: false,
            allowed_ciphers: vec![],
        })
    }
}
```

## File Permissions

```rust
pub fn set_secure_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        
        let metadata = std::fs::metadata(path)?;
        let mut permissions = metadata.permissions();
        
        // Owner read/write only (0600)
        permissions.set_mode(0o600);
        std::fs::set_permissions(path, permissions)?;
    }
    
    Ok(())
}

pub fn validate_recording_directory(path: &Path) -> Result<()> {
    // Must exist
    if !path.exists() {
        return Err(SecurityError::DirectoryNotFound);
    }
    
    // Must be directory
    if !path.is_dir() {
        return Err(SecurityError::NotADirectory);
    }
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        
        let metadata = std::fs::metadata(path)?;
        let permissions = metadata.permissions();
        let mode = permissions.mode();
        
        // Check not world-writable
        if mode & 0o002 != 0 {
            return Err(SecurityError::WorldWritable);
        }
    }
    
    Ok(())
}
```

## Configuration Validation

```rust
impl Config {
    pub fn validate_security(&self) -> Result<()> {
        // Ensure recording directory is secure
        validate_recording_directory(&self.recording_dir)?;
        
        // Validate redaction config
        if self.redaction.secrets.is_empty() && self.redaction.regex_patterns.is_empty() {
            warn!("No redaction patterns configured - secrets may leak!");
        }
        
        // Validate endpoint configs
        for endpoint in &self.endpoints {
            endpoint.validate_security()?;
        }
        
        // Check for overly permissive settings
        if self.limits.max_request_size > 100 * 1024 * 1024 {
            warn!("Very large max_request_size may enable DoS attacks");
        }
        
        if self.limits.max_connections > 10_000 {
            warn!("Very large max_connections may exhaust system resources");
        }
        
        Ok(())
    }
}
```

## Audit Logging

```rust
pub struct AuditLog {
    writer: Mutex<BufWriter<File>>,
}

pub struct AuditEvent {
    pub timestamp: SystemTime,
    pub event_type: EventType,
    pub source_ip: IpAddr,
    pub session_id: String,
    pub details: String,
}

pub enum EventType {
    RecordingCreated,
    RecordingAccessed,
    RecordingDeleted,
    RateLimitViolation,
    SecurityViolation,
    ConfigurationChange,
}

impl AuditLog {
    pub async fn log(&self, event: AuditEvent) -> Result<()> {
        let entry = serde_json::to_string(&event)?;
        
        let mut writer = self.writer.lock().await;
        writeln!(writer, "{}", entry)?;
        writer.flush()?;
        
        Ok(())
    }
}
```

## Secrets Management

```rust
pub fn load_secrets_from_env() -> Vec<String> {
    let mut secrets = Vec::new();
    
    // Load from environment variable (comma-separated)
    if let Ok(env_secrets) = std::env::var("OULI_SECRETS") {
        secrets.extend(
            env_secrets.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        );
    }
    
    // Load from file
    if let Ok(secrets_file) = std::env::var("OULI_SECRETS_FILE") {
        if let Ok(content) = std::fs::read_to_string(&secrets_file) {
            secrets.extend(
                content.lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && !s.starts_with('#'))
            );
        }
    }
    
    secrets
}

#[test]
fn test_secret_loading() {
    std::env::set_var("OULI_SECRETS", "secret1,secret2,secret3");
    let secrets = load_secrets_from_env();
    
    assert_eq!(secrets.len(), 3);
    assert!(secrets.contains(&"secret1".to_string()));
}
```

## Security Headers

```rust
pub fn add_security_headers(response: &mut Response<BoxBody>) {
    let headers = response.headers_mut();
    
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff")
    );
    
    headers.insert(
        "X-Frame-Options",
        HeaderValue::from_static("DENY")
    );
    
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static("1; mode=block")
    );
    
    headers.insert(
        "Strict-Transport-Security",
        HeaderValue::from_static("max-age=31536000; includeSubDomains")
    );
}
```

## Testing

```rust
#[test]
fn test_redaction() {
    let config = RedactionConfig {
        secrets: vec![
            "sk-1234567890abcdef".to_string(),
            "Bearer eyJhbGc...".to_string(),
        ],
        ..Default::default()
    };
    
    let redactor = Redactor::new(&config).unwrap();
    
    let input = "Authorization: Bearer eyJhbGc... and key=sk-1234567890abcdef";
    let output = redactor.redact_str(input);
    
    assert!(!output.contains("sk-1234567890abcdef"));
    assert!(!output.contains("eyJhbGc..."));
    assert!(output.contains("REDACTED"));
}

#[test]
fn test_json_redaction() {
    let redactor = Redactor::new(&RedactionConfig::default()).unwrap();
    
    let mut json = serde_json::json!({
        "username": "alice",
        "password": "secret123",
        "api_key": "xyz789",
        "data": {
            "token": "abc456"
        }
    });
    
    redactor.redact_json(&mut json);
    
    assert_eq!(json["password"], "REDACTED");
    assert_eq!(json["api_key"], "REDACTED");
    assert_eq!(json["data"]["token"], "REDACTED");
    assert_eq!(json["username"], "alice"); // Not redacted
}
```

## References

- [OWASP Top 10](https://owasp.org/www-project-top-ten/)
- [CWE-22: Path Traversal](https://cwe.mitre.org/data/definitions/22.html)
- [CWE-200: Information Exposure](https://cwe.mitre.org/data/definitions/200.html)
- [Boyer-Moore Algorithm](https://en.wikipedia.org/wiki/Boyer%E2%80%93Moore_string-search_algorithm)

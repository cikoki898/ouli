//! Request fingerprinting for deterministic hash generation

use sha2::{Digest, Sha256};

/// Special hash for chain head (first request in a session)
pub const CHAIN_HEAD_HASH: [u8; 32] = [
    0xb4, 0xd6, 0xe6, 0x0a, 0x9b, 0x97, 0xe7, 0xb9, 0x8c, 0x63, 0xdf, 0x93, 0x08, 0x72, 0x8c, 0x5c,
    0x88, 0xc0, 0xb4, 0x0c, 0x39, 0x80, 0x46, 0x77, 0x2c, 0x63, 0x44, 0x7b, 0x94, 0x60, 0x8b, 0x4d,
];

/// Simple request representation for fingerprinting
#[derive(Debug, Clone)]
pub struct Request {
    /// HTTP method (e.g., "GET", "POST")
    pub method: String,
    /// Request path
    pub path: String,
    /// Query parameters (sorted)
    pub query: Vec<(String, String)>,
    /// Headers (sorted)
    pub headers: Vec<(String, String)>,
    /// Request body
    pub body: Vec<u8>,
}

/// Compute SHA-256 fingerprint of a request
///
/// The fingerprint includes:
/// 1. Method (uppercase normalized)
/// 2. Path (normalized)
/// 3. Query parameters (sorted)
/// 4. Headers (sorted, normalized)
/// 5. Body
/// 6. Previous request hash (for chaining)
#[must_use]
pub fn fingerprint_request(request: &Request, prev_hash: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();

    // 1. Method (uppercase normalized)
    let method = request.method.to_uppercase();
    hasher.update((method.len() as u32).to_le_bytes());
    hasher.update(method.as_bytes());

    // 2. Path (normalized)
    let path = normalize_path(&request.path);
    hasher.update((path.len() as u32).to_le_bytes());
    hasher.update(path.as_bytes());

    // 3. Query parameters (sorted)
    let mut query = request.query.clone();
    query.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in &query {
        hasher.update((key.len() as u32).to_le_bytes());
        hasher.update(key.as_bytes());
        hasher.update((value.len() as u32).to_le_bytes());
        hasher.update(value.as_bytes());
    }

    // 4. Headers (sorted, normalized)
    let mut headers = request.headers.clone();
    headers.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    for (name, value) in &headers {
        let name_lower = name.to_lowercase();
        let value_trimmed = value.trim();
        hasher.update((name_lower.len() as u32).to_le_bytes());
        hasher.update(name_lower.as_bytes());
        hasher.update((value_trimmed.len() as u32).to_le_bytes());
        hasher.update(value_trimmed.as_bytes());
    }

    // 5. Body
    hasher.update((request.body.len() as u32).to_le_bytes());
    hasher.update(&request.body);

    // 6. Previous request hash (chain linkage)
    hasher.update(prev_hash);

    hasher.finalize().into()
}

/// Normalize a URL path
fn normalize_path(path: &str) -> String {
    // Remove leading/trailing whitespace
    let trimmed = path.trim();

    // Ensure leading slash
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        format!("/{trimmed}")
    } else {
        trimmed.to_string()
    }
}

/// Request chain tracker
#[derive(Debug, Clone, Copy)]
pub struct RequestChain {
    current_hash: [u8; 32],
}

impl RequestChain {
    /// Create a new request chain
    #[must_use]
    pub fn new() -> Self {
        Self {
            current_hash: CHAIN_HEAD_HASH,
        }
    }

    /// Create a chain from a stored hash
    #[must_use]
    pub fn from_hash(hash: [u8; 32]) -> Self {
        Self { current_hash: hash }
    }

    /// Process a request and return its hash
    pub fn process_request(&mut self, request: &Request) -> [u8; 32] {
        let hash = fingerprint_request(request, self.current_hash);
        self.current_hash = hash;
        hash
    }

    /// Get the current (previous) hash
    #[must_use]
    pub fn previous_hash(&self) -> [u8; 32] {
        self.current_hash
    }

    /// Get the current hash (for serialization)
    #[must_use]
    pub fn current_hash(&self) -> [u8; 32] {
        self.current_hash
    }

    /// Reset the chain
    pub fn reset(&mut self) {
        self.current_hash = CHAIN_HEAD_HASH;
    }
}

impl Default for RequestChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_request() -> Request {
        Request {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            query: vec![],
            headers: vec![],
            body: vec![],
        }
    }

    #[test]
    fn test_fingerprint_deterministic() {
        let request = test_request();
        let hash1 = fingerprint_request(&request, CHAIN_HEAD_HASH);
        let hash2 = fingerprint_request(&request, CHAIN_HEAD_HASH);

        assert_eq!(hash1, hash2, "Fingerprint must be deterministic");
    }

    #[test]
    fn test_fingerprint_different_methods() {
        let mut req1 = test_request();
        req1.method = "GET".to_string();

        let mut req2 = test_request();
        req2.method = "POST".to_string();

        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH);
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH);

        assert_ne!(
            hash1, hash2,
            "Different methods should produce different hashes"
        );
    }

    #[test]
    fn test_fingerprint_different_paths() {
        let mut req1 = test_request();
        req1.path = "/api/v1".to_string();

        let mut req2 = test_request();
        req2.path = "/api/v2".to_string();

        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH);
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH);

        assert_ne!(
            hash1, hash2,
            "Different paths should produce different hashes"
        );
    }

    #[test]
    fn test_header_order_independence() {
        let mut req1 = test_request();
        req1.headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
        ];

        let mut req2 = test_request();
        req2.headers = vec![
            ("Accept".to_string(), "application/json".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH);
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH);

        assert_eq!(hash1, hash2, "Header order should not affect fingerprint");
    }

    #[test]
    fn test_header_case_insensitivity() {
        let mut req1 = test_request();
        req1.headers = vec![("Content-Type".to_string(), "application/json".to_string())];

        let mut req2 = test_request();
        req2.headers = vec![("content-type".to_string(), "application/json".to_string())];

        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH);
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH);

        assert_eq!(hash1, hash2, "Header names should be case-insensitive");
    }

    #[test]
    fn test_query_order_independence() {
        let mut req1 = test_request();
        req1.query = vec![
            ("b".to_string(), "2".to_string()),
            ("a".to_string(), "1".to_string()),
        ];

        let mut req2 = test_request();
        req2.query = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ];

        let hash1 = fingerprint_request(&req1, CHAIN_HEAD_HASH);
        let hash2 = fingerprint_request(&req2, CHAIN_HEAD_HASH);

        assert_eq!(
            hash1, hash2,
            "Query parameter order should not affect fingerprint"
        );
    }

    #[test]
    fn test_request_chain() {
        let mut chain = RequestChain::new();

        let req1 = test_request();
        let hash1 = chain.process_request(&req1);

        let req2 = test_request();
        let hash2 = chain.process_request(&req2);

        // Same request should have different hashes due to chain
        assert_ne!(hash1, hash2, "Chain should link requests");

        // Previous hash should be the first request's hash
        assert_eq!(chain.previous_hash(), hash2);
    }

    #[test]
    fn test_chain_reset() {
        let mut chain = RequestChain::new();

        let req = test_request();
        let hash1 = chain.process_request(&req);

        chain.reset();

        let hash2 = chain.process_request(&req);

        // After reset, same request should produce same hash
        assert_eq!(hash1, hash2, "Reset should restart chain");
    }

    #[test]
    fn test_path_normalization() {
        assert_eq!(normalize_path("/api/test"), "/api/test");
        assert_eq!(normalize_path("api/test"), "/api/test");
        assert_eq!(normalize_path("  /api/test  "), "/api/test");
        assert_eq!(normalize_path(""), "/");
    }
}

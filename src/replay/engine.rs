//! Replay engine for serving recorded responses

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{debug, info, warn};

use crate::fingerprint::{fingerprint_request, Request};
use crate::{OuliError, Result};

use super::cache::{CachedResponse, ReplayCache};
use super::WarmingStrategy;

/// Replay engine for serving recorded responses
pub struct ReplayEngine {
    cache: Arc<ReplayCache>,
}

impl ReplayEngine {
    /// Create a new replay engine
    #[must_use]
    pub fn new(recording_dir: PathBuf, warming_strategy: WarmingStrategy) -> Self {
        Self {
            cache: Arc::new(ReplayCache::new(recording_dir, warming_strategy)),
        }
    }

    /// Warm the cache
    ///
    /// # Errors
    ///
    /// Returns error if warming fails
    pub fn warm(&self) -> Result<()> {
        self.cache.warm()
    }

    /// Load a specific recording
    ///
    /// # Errors
    ///
    /// Returns error if recording cannot be loaded
    pub fn load_recording(&self, test_name: &str) -> Result<()> {
        self.cache.load_recording(test_name)
    }

    /// Replay a request and get the cached response
    ///
    /// # Errors
    ///
    /// Returns error if response not found
    pub fn replay_request(
        &self,
        method: String,
        path: String,
        query: Vec<(String, String)>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        prev_hash: [u8; 32],
    ) -> Result<CachedResponse> {
        // Build request for fingerprinting
        let request = Request {
            method,
            path,
            query,
            headers,
            body,
        };

        // Compute fingerprint
        let request_hash = fingerprint_request(&request, prev_hash);

        debug!(
            "Replaying request: {} (hash: {})",
            request.method,
            hex::encode(&request_hash[..8])
        );

        // Look up in cache
        if let Some(response) = self.cache.lookup(request_hash) {
            debug!(
                "Cache hit: {} {} -> {}",
                request.method, request.path, response.status
            );
            Ok(response)
        } else {
            warn!(
                "Cache miss: {} {} (hash: {})",
                request.method,
                request.path,
                hex::encode(&request_hash[..8])
            );
            Err(OuliError::RecordingNotFound(request_hash))
        }
    }

    /// Get cache statistics
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            hits: self.cache.hit_count(),
            misses: self.cache.miss_count(),
            hit_rate: self.cache.hit_rate(),
            size: self.cache.size(),
        }
    }

    /// Clear the cache
    pub fn clear_cache(&self) {
        info!("Clearing replay cache");
        self.cache.clear();
    }
}

/// Cache statistics
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Cache hits
    pub hits: usize,
    /// Cache misses
    pub misses: usize,
    /// Hit rate (0.0 to 1.0)
    pub hit_rate: f64,
    /// Cache size (number of entries)
    pub size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_replay_engine_creation() {
        let temp_dir = TempDir::new().unwrap();
        let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        let stats = engine.cache_stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.size, 0);
    }

    #[test]
    fn test_replay_request_miss() {
        let temp_dir = TempDir::new().unwrap();
        let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        let result = engine.replay_request(
            "GET".to_string(),
            "/test".to_string(),
            vec![],
            vec![],
            vec![],
            [0u8; 32],
        );

        assert!(result.is_err());

        let stats = engine.cache_stats();
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_cache_clear() {
        let temp_dir = TempDir::new().unwrap();
        let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        // Try a request (will miss)
        let _ = engine.replay_request(
            "GET".to_string(),
            "/test".to_string(),
            vec![],
            vec![],
            vec![],
            [0u8; 32],
        );

        assert_eq!(engine.cache_stats().misses, 1);

        engine.clear_cache();

        assert_eq!(engine.cache_stats().misses, 0);
    }
}

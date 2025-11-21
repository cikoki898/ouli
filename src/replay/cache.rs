//! Replay cache for fast request/response lookup

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use tracing::{debug, info, warn};

use crate::storage::RecordingReader;
use crate::{OuliError, Result};

use super::WarmingStrategy;

/// Cached response data
#[derive(Clone)]
pub struct CachedResponse {
    /// Response status code
    pub status: u16,
    /// Response headers
    pub headers: Vec<(String, String)>,
    /// Response body
    pub body: Vec<u8>,
}

/// Replay cache for fast lookups
pub struct ReplayCache {
    /// Map of request hash to response
    cache: DashMap<[u8; 32], CachedResponse>,
    /// Map of test name to recording file path
    recordings: DashMap<String, PathBuf>,
    /// Cache hit counter
    hits: AtomicUsize,
    /// Cache miss counter
    misses: AtomicUsize,
    /// Recording directory
    recording_dir: PathBuf,
    /// Warming strategy
    warming_strategy: WarmingStrategy,
}

impl ReplayCache {
    /// Create a new replay cache
    #[must_use]
    pub fn new(recording_dir: PathBuf, warming_strategy: WarmingStrategy) -> Self {
        Self {
            cache: DashMap::new(),
            recordings: DashMap::new(),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            recording_dir,
            warming_strategy,
        }
    }

    /// Load a recording into the cache
    ///
    /// # Errors
    ///
    /// Returns error if recording cannot be loaded
    pub fn load_recording(&self, test_name: &str) -> Result<()> {
        let file_path = self.recording_dir.join(format!("{test_name}.ouli"));

        if !file_path.exists() {
            return Err(OuliError::FileNotFound(file_path.display().to_string()));
        }

        debug!("Loading recording: {}", test_name);

        let reader = RecordingReader::open(&file_path)?;
        let mut loaded_count = 0;

        // Stream interactions without allocating intermediate Vec
        // This is more memory-efficient for large recordings
        for entry in reader.entries_iter() {
            // Deserialize response
            if let Ok(response_data) = reader.read_response(&entry) {
                if let Ok(response) = deserialize_response(response_data) {
                    self.cache.insert(entry.request_hash, response);
                    loaded_count += 1;
                }
            }
        }

        self.recordings
            .insert(test_name.to_string(), file_path.clone());

        info!(
            "Loaded recording '{}': {} interactions",
            test_name, loaded_count
        );

        Ok(())
    }

    /// Load all recordings from the directory
    ///
    /// # Errors
    ///
    /// Returns error if directory cannot be read
    pub fn load_all_recordings(&self) -> Result<()> {
        let entries = std::fs::read_dir(&self.recording_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("ouli") {
                if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Err(e) = self.load_recording(file_name) {
                        warn!("Failed to load recording '{}': {}", file_name, e);
                    }
                }
            }
        }

        Ok(())
    }

    /// Look up a response by request hash
    #[must_use]
    pub fn lookup(&self, request_hash: [u8; 32]) -> Option<CachedResponse> {
        if let Some(response) = self.cache.get(&request_hash) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(response.clone())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Warm the cache based on strategy
    ///
    /// # Errors
    ///
    /// Returns error if warming fails
    pub fn warm(&self) -> Result<()> {
        match self.warming_strategy {
            WarmingStrategy::Eager => {
                info!("Warming cache eagerly");
                self.load_all_recordings()?;
            }
            WarmingStrategy::Lazy => {
                debug!("Using lazy cache warming");
            }
        }

        Ok(())
    }

    /// Get cache hit count
    #[must_use]
    pub fn hit_count(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get cache miss count
    #[must_use]
    pub fn miss_count(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get cache hit rate (0.0 to 1.0)
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hit_count();
        let misses = self.miss_count();
        let total = hits + misses;

        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get the number of cached responses
    #[must_use]
    pub fn size(&self) -> usize {
        self.cache.len()
    }

    /// Clear the cache
    pub fn clear(&self) {
        self.cache.clear();
        self.recordings.clear();
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }
}

/// Deserialize a response from storage
fn deserialize_response(data: &[u8]) -> Result<CachedResponse> {
    // Simple deserialization matching the recording format
    let mut offset = 0;

    // Status (2 bytes)
    if data.len() < 2 {
        return Err(OuliError::InvalidFormat("Response too short".to_string()));
    }
    let status = u16::from_le_bytes([data[0], data[1]]);
    offset += 2;

    // Headers count (2 bytes)
    if data.len() < offset + 2 {
        return Err(OuliError::InvalidFormat(
            "Missing headers count".to_string(),
        ));
    }
    let header_count = u16::from_le_bytes([data[offset], data[offset + 1]]);
    offset += 2;

    let mut headers = Vec::new();
    for _ in 0..header_count {
        // Name length
        if data.len() < offset + 2 {
            return Err(OuliError::InvalidFormat(
                "Missing header name length".to_string(),
            ));
        }
        let name_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        // Name
        if data.len() < offset + name_len {
            return Err(OuliError::InvalidFormat("Missing header name".to_string()));
        }
        let name = String::from_utf8_lossy(&data[offset..offset + name_len]).to_string();
        offset += name_len;

        // Value length
        if data.len() < offset + 2 {
            return Err(OuliError::InvalidFormat(
                "Missing header value length".to_string(),
            ));
        }
        let value_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        // Value
        if data.len() < offset + value_len {
            return Err(OuliError::InvalidFormat("Missing header value".to_string()));
        }
        let value = String::from_utf8_lossy(&data[offset..offset + value_len]).to_string();
        offset += value_len;

        headers.push((name, value));
    }

    // Body length (4 bytes)
    if data.len() < offset + 4 {
        return Err(OuliError::InvalidFormat("Missing body length".to_string()));
    }
    let body_len = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;
    offset += 4;

    // Body
    if data.len() < offset + body_len {
        return Err(OuliError::InvalidFormat("Missing body".to_string()));
    }
    let body = data[offset..offset + body_len].to_vec();

    Ok(CachedResponse {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_cache_creation() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ReplayCache::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        assert_eq!(cache.size(), 0);
        assert_eq!(cache.hit_count(), 0);
        assert_eq!(cache.miss_count(), 0);
    }

    #[test]
    fn test_cache_lookup_miss() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ReplayCache::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        let hash = [1u8; 32];
        assert!(cache.lookup(hash).is_none());
        assert_eq!(cache.miss_count(), 1);
    }

    #[test]
    fn test_cache_metrics() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ReplayCache::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        // Insert a response manually for testing
        let hash = [1u8; 32];
        cache.cache.insert(
            hash,
            CachedResponse {
                status: 200,
                headers: vec![],
                body: vec![],
            },
        );

        // Hit
        assert!(cache.lookup(hash).is_some());
        assert_eq!(cache.hit_count(), 1);

        // Miss
        let hash2 = [2u8; 32];
        assert!(cache.lookup(hash2).is_none());
        assert_eq!(cache.miss_count(), 1);

        // Hit rate
        assert!((cache.hit_rate() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_cache_clear() {
        let temp_dir = TempDir::new().unwrap();
        let cache = ReplayCache::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        let hash = [1u8; 32];
        cache.cache.insert(
            hash,
            CachedResponse {
                status: 200,
                headers: vec![],
                body: vec![],
            },
        );

        assert_eq!(cache.size(), 1);

        cache.clear();

        assert_eq!(cache.size(), 0);
        assert_eq!(cache.hit_count(), 0);
        assert_eq!(cache.miss_count(), 0);
    }

    #[test]
    fn test_deserialize_response() {
        let mut data = Vec::new();

        // Status: 200
        data.extend_from_slice(&200u16.to_le_bytes());

        // Headers: 1 header
        data.extend_from_slice(&1u16.to_le_bytes());

        // Header name: "Content-Type" (12 bytes)
        data.extend_from_slice(&12u16.to_le_bytes());
        data.extend_from_slice(b"Content-Type");

        // Header value: "text/plain" (10 bytes)
        data.extend_from_slice(&10u16.to_le_bytes());
        data.extend_from_slice(b"text/plain");

        // Body: "Hello" (5 bytes)
        data.extend_from_slice(&5u32.to_le_bytes());
        data.extend_from_slice(b"Hello");

        let response = deserialize_response(&data).unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.headers.len(), 1);
        assert_eq!(response.headers[0].0, "Content-Type");
        assert_eq!(response.headers[0].1, "text/plain");
        assert_eq!(response.body, b"Hello");
    }
}

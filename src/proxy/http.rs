//! HTTP proxy with recording and replay

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::config::{Config, Mode};
use crate::fingerprint::{self, RequestChain};
use crate::recording::{RecordingEngine, Response as RecordResponse};
use crate::replay::ReplayEngine;
use crate::{OuliError, Result};

/// HTTP proxy that handles recording and replay
pub struct HttpProxy {
    config: Arc<Config>,
    recording_engine: Option<Arc<RecordingEngine>>,
    replay_engine: Option<Arc<ReplayEngine>>,
    request_chain: Arc<RwLock<RequestChain>>,
}

impl HttpProxy {
    /// Create a new HTTP proxy
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        let recording_engine = if config.mode.is_record() {
            Some(Arc::new(RecordingEngine::new(config.recording_dir.clone())))
        } else {
            None
        };

        let replay_engine = if config.mode.is_replay() {
            Some(Arc::new(ReplayEngine::new(
                config.recording_dir.clone(),
                crate::replay::WarmingStrategy::Lazy,
            )))
        } else {
            None
        };

        Self {
            config,
            recording_engine,
            replay_engine,
            request_chain: Arc::new(RwLock::new(RequestChain::new())),
        }
    }

    /// Handle an HTTP request in record or replay mode
    ///
    /// # Errors
    ///
    /// Returns error if proxying fails
    pub async fn handle_request(
        &self,
        method: String,
        path: String,
        query: Vec<(String, String)>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<RecordResponse> {
        match self.config.mode {
            Mode::Record => self.handle_record(method, path, query, headers, body).await,
            Mode::Replay => self.handle_replay(method, path, query, headers, body).await,
        }
    }

    /// Handle request in record mode
    async fn handle_record(
        &self,
        method: String,
        path: String,
        query: Vec<(String, String)>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<RecordResponse> {
        debug!("Record mode: {} {}", method, path);

        // TODO: Forward request to target endpoint
        // For Milestone 5, return a mock response
        let response = RecordResponse {
            status: 200,
            headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
            body: b"Mock response from record mode".to_vec(),
        };

        // Build request for recording
        let request = fingerprint::Request {
            method,
            path,
            query,
            headers,
            body,
        };

        // Record the interaction
        if let Some(ref engine) = self.recording_engine {
            engine
                .record_interaction(None, request, response.clone())
                .await?;
        }

        Ok(response)
    }

    /// Handle request in replay mode
    async fn handle_replay(
        &self,
        method: String,
        path: String,
        query: Vec<(String, String)>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<RecordResponse> {
        debug!("Replay mode: {} {}", method, path);

        // Get previous hash from chain
        let prev_hash = {
            let chain = self.request_chain.read().await;
            chain.previous_hash()
        };

        // Try to replay from cache
        if let Some(ref engine) = self.replay_engine {
            match engine.replay_request(method, path, query, headers, body, prev_hash) {
                Ok(cached) => {
                    info!("Replay cache hit");
                    Ok(RecordResponse {
                        status: cached.status,
                        headers: cached.headers,
                        body: cached.body,
                    })
                }
                Err(OuliError::RecordingNotFound(hash)) => {
                    warn!("Replay cache miss: {}", hex::encode(&hash[..8]));
                    Err(OuliError::RecordingNotFound(hash))
                }
                Err(e) => Err(e),
            }
        } else {
            Err(OuliError::Other(
                "Replay engine not initialized".to_string(),
            ))
        }
    }

    /// Finalize recording (if in record mode)
    ///
    /// # Errors
    ///
    /// Returns error if finalization fails
    pub async fn finalize(&self) -> Result<()> {
        if let Some(ref engine) = self.recording_engine {
            info!("Finalizing recording sessions");
            engine.finalize_all().await?;
        }
        Ok(())
    }

    /// Warm replay cache (if in replay mode)
    ///
    /// # Errors
    ///
    /// Returns error if warming fails
    pub fn warm_cache(&self) -> Result<()> {
        if let Some(ref engine) = self.replay_engine {
            info!("Warming replay cache");
            engine.warm()?;
        }
        Ok(())
    }

    /// Load a specific recording into replay cache
    ///
    /// # Errors
    ///
    /// Returns error if loading fails
    pub fn load_recording(&self, test_name: &str) -> Result<()> {
        if let Some(ref engine) = self.replay_engine {
            info!("Loading recording: {}", test_name);
            engine.load_recording(test_name)?;
        }
        Ok(())
    }

    /// Get cache statistics (replay mode only)
    #[must_use]
    pub fn cache_stats(&self) -> Option<crate::replay::CacheStats> {
        self.replay_engine.as_ref().map(|e| e.cache_stats())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EndpointConfig, LimitsConfig, RedactionConfig};
    use tempfile::TempDir;

    fn create_test_config(mode: Mode, temp_dir: &TempDir) -> Config {
        Config {
            mode,
            recording_dir: temp_dir.path().to_path_buf(),
            endpoints: vec![EndpointConfig {
                target_host: "example.com".to_string(),
                target_port: 443,
                source_port: 8080,
                target_type: "https".to_string(),
                source_type: "http".to_string(),
                redact_request_headers: vec![],
            }],
            redaction: RedactionConfig::default(),
            limits: LimitsConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_proxy_creation_record_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Record, &temp_dir));
        let proxy = HttpProxy::new(config);

        assert!(proxy.recording_engine.is_some());
        assert!(proxy.replay_engine.is_none());
    }

    #[tokio::test]
    async fn test_proxy_creation_replay_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Replay, &temp_dir));
        let proxy = HttpProxy::new(config);

        assert!(proxy.recording_engine.is_none());
        assert!(proxy.replay_engine.is_some());
    }

    #[tokio::test]
    async fn test_handle_request_record_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Record, &temp_dir));
        let proxy = HttpProxy::new(config);

        let result = proxy
            .handle_request(
                "GET".to_string(),
                "/test".to_string(),
                vec![],
                vec![],
                vec![],
            )
            .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_handle_request_replay_mode_miss() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Replay, &temp_dir));
        let proxy = HttpProxy::new(config);

        let result = proxy
            .handle_request(
                "GET".to_string(),
                "/test".to_string(),
                vec![],
                vec![],
                vec![],
            )
            .await;

        // Should miss since cache is empty
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_finalize_record_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Record, &temp_dir));
        let proxy = HttpProxy::new(config);

        let result = proxy.finalize().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cache_stats_replay_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Replay, &temp_dir));
        let proxy = HttpProxy::new(config);

        let stats = proxy.cache_stats();
        assert!(stats.is_some());
    }

    #[tokio::test]
    async fn test_cache_stats_record_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = Arc::new(create_test_config(Mode::Record, &temp_dir));
        let proxy = HttpProxy::new(config);

        let stats = proxy.cache_stats();
        assert!(stats.is_none());
    }
}

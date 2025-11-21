//! Integration tests for record-replay cycle

use std::sync::Arc;
use tempfile::TempDir;

use ouli::config::{Config, EndpointConfig, LimitsConfig, Mode, RedactionConfig};
use ouli::fingerprint::{Request, CHAIN_HEAD_HASH};
use ouli::proxy::HttpProxy;
use ouli::recording::{RecordingEngine, Response};
use ouli::replay::{ReplayEngine, WarmingStrategy};

/// Create test configuration
fn create_test_config(mode: Mode, recording_dir: std::path::PathBuf) -> Config {
    Config {
        mode,
        recording_dir,
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

// NOTE: This test makes real HTTP requests
#[tokio::test]
#[ignore = "makes real HTTP requests"]
async fn test_record_and_replay_single_request() {
    let temp_dir = TempDir::new().unwrap();

    // Phase 1: Record mode
    {
        let config = Arc::new(create_test_config(
            Mode::Record,
            temp_dir.path().to_path_buf(),
        ));
        let proxy = HttpProxy::new(config);

        // Record ONE request
        let response = proxy
            .handle_request(
                "GET".to_string(),
                "/api/test".to_string(),
                vec![],
                vec![],
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(response.status, 200);

        // Finalize recordings
        proxy.finalize().await.unwrap();
    }

    // Phase 2: Replay mode
    {
        // Check that recording file exists
        let recording_file = temp_dir.path().join("default.ouli");
        assert!(recording_file.exists(), "Recording file should exist");

        let config = Arc::new(create_test_config(
            Mode::Replay,
            temp_dir.path().to_path_buf(),
        ));
        let proxy = HttpProxy::new(config);

        // Warm cache
        proxy.warm_cache().unwrap();

        // Check cache was populated
        let stats_before = proxy.cache_stats().unwrap();
        assert!(
            stats_before.size > 0,
            "Cache should have entries after warming"
        );

        // Replay the request
        let response = proxy
            .handle_request(
                "GET".to_string(),
                "/api/test".to_string(),
                vec![],
                vec![],
                vec![],
            )
            .await
            .unwrap();

        assert_eq!(response.status, 200);

        // Check cache stats
        let stats = proxy.cache_stats().unwrap();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);
    }
}

#[tokio::test]
async fn test_recording_engine_direct() {
    let temp_dir = TempDir::new().unwrap();
    let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

    // Record multiple interactions
    for i in 0..10 {
        let request = Request {
            method: "GET".to_string(),
            path: format!("/api/test/{}", i),
            query: vec![],
            headers: vec![],
            body: vec![],
        };

        let response = Response {
            status: 200,
            headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
            body: format!("Response {}", i).into_bytes(),
        };

        engine
            .record_interaction(None, request, response)
            .await
            .unwrap();
    }

    // Verify session count
    assert_eq!(engine.session_count(), 1);

    // Finalize
    engine.finalize_all().await.unwrap();
    assert_eq!(engine.session_count(), 0);
}

#[tokio::test]
async fn test_replay_engine_direct() {
    let temp_dir = TempDir::new().unwrap();

    // First record some data
    {
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        let request = Request {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            query: vec![("key".to_string(), "value".to_string())],
            headers: vec![("Accept".to_string(), "application/json".to_string())],
            body: vec![],
        };

        let response = Response {
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: b"{\"status\":\"ok\"}".to_vec(),
        };

        engine
            .record_interaction(Some("test_replay"), request, response)
            .await
            .unwrap();

        engine.finalize_all().await.unwrap();
    }

    // Now replay
    {
        let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Eager);

        // Warm cache
        engine.warm().unwrap();

        // Replay the request (use CHAIN_HEAD_HASH as prev_hash for first request)
        let response = engine
            .replay_request(
                "GET".to_string(),
                "/api/test".to_string(),
                vec![("key".to_string(), "value".to_string())],
                vec![("Accept".to_string(), "application/json".to_string())],
                vec![],
                CHAIN_HEAD_HASH,
            )
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"status\":\"ok\"}");

        // Check stats
        let stats = engine.cache_stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);
    }
}

#[tokio::test]
async fn test_multiple_sessions() {
    let temp_dir = TempDir::new().unwrap();
    let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

    // Record in different sessions
    for session_num in 0..5 {
        for request_num in 0..3 {
            let request = Request {
                method: "GET".to_string(),
                path: format!("/session{}/request{}", session_num, request_num),
                query: vec![],
                headers: vec![],
                body: vec![],
            };

            let response = Response {
                status: 200,
                headers: vec![],
                body: format!("Session {} Request {}", session_num, request_num).into_bytes(),
            };

            engine
                .record_interaction(Some(&format!("session_{}", session_num)), request, response)
                .await
                .unwrap();
        }
    }

    // Verify session count
    assert_eq!(engine.session_count(), 5);

    // Finalize all
    engine.finalize_all().await.unwrap();
    assert_eq!(engine.session_count(), 0);
}

#[tokio::test]
async fn test_replay_cache_miss() {
    let temp_dir = TempDir::new().unwrap();
    let config = Arc::new(create_test_config(
        Mode::Replay,
        temp_dir.path().to_path_buf(),
    ));
    let proxy = HttpProxy::new(config);

    // Try to replay without any recordings
    let result = proxy
        .handle_request(
            "GET".to_string(),
            "/nonexistent".to_string(),
            vec![],
            vec![],
            vec![],
        )
        .await;

    // Should get a recording not found error
    assert!(result.is_err());
}

#[tokio::test]
async fn test_lazy_vs_eager_warming() {
    let temp_dir = TempDir::new().unwrap();

    // Record some data
    {
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        let request = Request {
            method: "GET".to_string(),
            path: "/api/test".to_string(),
            query: vec![],
            headers: vec![],
            body: vec![],
        };

        let response = Response {
            status: 200,
            headers: vec![],
            body: b"test".to_vec(),
        };

        engine
            .record_interaction(Some("lazy_eager_test"), request, response)
            .await
            .unwrap();

        engine.finalize_all().await.unwrap();
    }

    // Test eager warming
    {
        let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Eager);

        engine.warm().unwrap();

        let stats = engine.cache_stats();
        assert!(
            stats.size > 0,
            "Cache should be populated with eager warming"
        );
    }

    // Test lazy warming
    {
        let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Lazy);

        engine.warm().unwrap();

        let stats = engine.cache_stats();
        assert_eq!(stats.size, 0, "Cache should be empty with lazy warming");

        // Load specific recording
        engine.load_recording("lazy_eager_test").unwrap();

        let stats = engine.cache_stats();
        assert!(stats.size > 0, "Cache should be populated after loading");
    }
}

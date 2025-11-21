//! Benchmarks for record-replay performance

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use tempfile::TempDir;

use ouli::config::{Config, EndpointConfig, LimitsConfig, Mode, RedactionConfig};
use ouli::fingerprint::Request;
use ouli::proxy::HttpProxy;
use ouli::recording::{RecordingEngine, Response};
use ouli::replay::{ReplayEngine, WarmingStrategy};

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

fn bench_record_single_request(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("record_single_request", |b| {
        b.iter(|| {
            rt.block_on(async {
                let temp_dir = TempDir::new().unwrap();
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
                    .record_interaction(None, request, response)
                    .await
                    .unwrap();

                engine.finalize_all().await.unwrap();
            });
        });
    });
}

fn bench_replay_single_request(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Setup: Record a single request
    let temp_dir = TempDir::new().unwrap();
    rt.block_on(async {
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
            .record_interaction(Some("bench_replay"), request, response)
            .await
            .unwrap();

        engine.finalize_all().await.unwrap();
    });

    let engine = ReplayEngine::new(temp_dir.path().to_path_buf(), WarmingStrategy::Eager);
    engine.warm().unwrap();

    c.bench_function("replay_single_request", |b| {
        b.iter(|| {
            let result = engine
                .replay_request(
                    black_box("GET".to_string()),
                    black_box("/api/test".to_string()),
                    black_box(vec![]),
                    black_box(vec![]),
                    black_box(vec![]),
                    black_box([0u8; 32]),
                )
                .unwrap();

            black_box(result);
        });
    });
}

fn bench_record_batch(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("record_100_requests", |b| {
        b.iter(|| {
            rt.block_on(async {
                let temp_dir = TempDir::new().unwrap();
                let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

                for i in 0..100 {
                    let request = Request {
                        method: "GET".to_string(),
                        path: format!("/api/test/{}", i),
                        query: vec![],
                        headers: vec![],
                        body: vec![],
                    };

                    let response = Response {
                        status: 200,
                        headers: vec![],
                        body: format!("Response {}", i).into_bytes(),
                    };

                    engine
                        .record_interaction(None, request, response)
                        .await
                        .unwrap();
                }

                engine.finalize_all().await.unwrap();
            });
        });
    });
}

fn bench_proxy_record(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("proxy_record_request", |b| {
        b.iter(|| {
            rt.block_on(async {
                let temp_dir = TempDir::new().unwrap();
                let config = Arc::new(create_test_config(
                    Mode::Record,
                    temp_dir.path().to_path_buf(),
                ));
                let proxy = HttpProxy::new(config);

                let response = proxy
                    .handle_request(
                        black_box("GET".to_string()),
                        black_box("/api/test".to_string()),
                        black_box(vec![]),
                        black_box(vec![]),
                        black_box(vec![]),
                    )
                    .await
                    .unwrap();

                black_box(response);

                proxy.finalize().await.unwrap();
            });
        });
    });
}

fn bench_cache_warming(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Setup: Record 100 requests
    let temp_dir = TempDir::new().unwrap();
    rt.block_on(async {
        let engine = RecordingEngine::new(temp_dir.path().to_path_buf());

        for i in 0..100 {
            let request = Request {
                method: "GET".to_string(),
                path: format!("/api/test/{}", i),
                query: vec![],
                headers: vec![],
                body: vec![],
            };

            let response = Response {
                status: 200,
                headers: vec![],
                body: format!("Response {}", i).into_bytes(),
            };

            engine
                .record_interaction(Some("cache_warming_bench"), request, response)
                .await
                .unwrap();
        }

        engine.finalize_all().await.unwrap();
    });

    c.bench_function("cache_warm_100_requests", |b| {
        b.iter(|| {
            let engine = ReplayEngine::new(
                black_box(temp_dir.path().to_path_buf()),
                WarmingStrategy::Eager,
            );
            engine.warm().unwrap();
            black_box(engine.cache_stats());
        });
    });
}

criterion_group!(
    benches,
    bench_record_single_request,
    bench_replay_single_request,
    bench_record_batch,
    bench_proxy_record,
    bench_cache_warming
);
criterion_main!(benches);

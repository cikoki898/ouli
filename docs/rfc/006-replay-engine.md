# RFC-006: Replay Engine

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Design the replay engine that serves recorded responses with sub-100μs latency through memory-mapped zero-copy reads and intelligent caching.

## Architecture

```rust
pub struct ReplayEngine {
    storage: Arc<StorageEngine>,
    cache: Arc<ReplayCache>,
    redactor: Arc<Redactor>,
    chain_tracker: DashMap<String, RequestChain>,
    config: Arc<Config>,
}

pub struct ReplayCache {
    recordings: Moka<String, Arc<RecordingReader>>,
    hot_responses: Moka<[u8; 32], Arc<CachedResponse>>,
}

pub struct CachedResponse {
    status: u16,
    headers: HeaderMap,
    body: Bytes, // Zero-copy from mmap
    chunks: Option<Vec<Bytes>>, // For streaming
}
```

## Request Flow

```rust
impl ReplayEngine {
    pub async fn replay_interaction(
        &self,
        request: Request<Incoming>,
        endpoint: &EndpointConfig,
    ) -> Result<Response<BoxBody>> {
        // 1. Extract and redact request
        let (parts, body) = request.into_parts();
        let body_bytes = self.read_body(body).await?;
        let redacted_request = self.redact_request(&parts, &body_bytes)?;
        
        // 2. Get session ID
        let session_id = self.get_session_id(&parts)?;
        
        // 3. Compute fingerprint
        let mut chain = self.get_or_create_chain(&session_id);
        let request_hash = chain.process_request(&redacted_request, &self.redactor);
        
        // 4. Lookup response
        let response = self.lookup_response(&session_id, request_hash).await?;
        
        // 5. Build and return response
        let mut builder = Response::builder().status(response.status);
        
        for (name, value) in &response.headers {
            builder = builder.header(name, value);
        }
        
        let body = if let Some(chunks) = &response.chunks {
            // Streaming response
            BoxBody::new(self.create_streaming_body(chunks.clone()))
        } else {
            // Simple response
            BoxBody::new(Full::new(response.body.clone()))
        };
        
        Ok(builder.body(body)?)
    }
}
```

## Zero-Copy Lookup

```rust
async fn lookup_response(
    &self,
    session_id: &str,
    request_hash: [u8; 32],
) -> Result<Arc<CachedResponse>> {
    // Check hot cache first (in-memory)
    if let Some(cached) = self.cache.hot_responses.get(&request_hash) {
        return Ok(cached);
    }
    
    // Load recording if not already loaded
    let reader = self.get_or_load_recording(session_id).await?;
    
    // Lookup interaction by hash
    let entry = reader.lookup(request_hash)
        .ok_or(OuliError::RecordingNotFound(request_hash))?;
    
    // Zero-copy read from mmap
    let response = reader.read_response(entry)?;
    
    // Cache for future use
    let cached = Arc::new(CachedResponse {
        status: response.status,
        headers: response.headers,
        body: response.body, // Bytes::from mmap slice (refcounted)
        chunks: response.chunks,
    });
    
    self.cache.hot_responses.insert(request_hash, cached.clone());
    
    Ok(cached)
}
```

## Recording Cache

```rust
async fn get_or_load_recording(&self, session_id: &str) -> Result<Arc<RecordingReader>> {
    // Check cache
    if let Some(reader) = self.cache.recordings.get(session_id) {
        return Ok(reader);
    }
    
    // Load from disk
    let path = self.storage.recording_path(session_id);
    
    if !path.exists() {
        return Err(OuliError::RecordingFileNotFound(session_id.to_string()));
    }
    
    let reader = Arc::new(RecordingReader::open(&path)?);
    
    // Cache the reader
    self.cache.recordings.insert(session_id.to_string(), reader.clone());
    
    Ok(reader)
}
```

## Memory-Mapped Reading

```rust
impl RecordingReader {
    pub fn read_response(&self, entry: &InteractionEntry) -> Result<ResponseData> {
        // Direct slice into mmap (zero-copy)
        let response_slice = &self.mmap[
            entry.response_offset as usize..
            (entry.response_offset + entry.response_size as u64) as usize
        ];
        
        // Parse response metadata
        let header: ResponseHeader = *bytemuck::from_bytes(&response_slice[0..24]);
        
        // Verify CRC
        let computed_crc = crc32(&response_slice[24..]);
        assert_eq!(header.crc, computed_crc, "Response data corrupted");
        
        // Parse headers
        let mut offset = 24;
        let mut headers = HeaderMap::new();
        
        for _ in 0..header.header_count {
            let name_len = u16::from_le_bytes([
                response_slice[offset],
                response_slice[offset + 1]
            ]) as usize;
            offset += 2;
            
            let value_len = u16::from_le_bytes([
                response_slice[offset],
                response_slice[offset + 1]
            ]) as usize;
            offset += 2;
            
            let name = &response_slice[offset..offset + name_len];
            offset += name_len;
            
            let value = &response_slice[offset..offset + value_len];
            offset += value_len;
            
            headers.insert(
                HeaderName::from_bytes(name)?,
                HeaderValue::from_bytes(value)?
            );
        }
        
        // Body is remaining bytes (zero-copy via Bytes::from)
        let body = if header.chunk_count > 0 {
            // Streaming: parse chunks
            let mut chunks = Vec::new();
            
            for _ in 0..header.chunk_count {
                let chunk_len = u32::from_le_bytes([
                    response_slice[offset],
                    response_slice[offset + 1],
                    response_slice[offset + 2],
                    response_slice[offset + 3],
                ]) as usize;
                offset += 4;
                
                let chunk = Bytes::copy_from_slice(&response_slice[offset..offset + chunk_len]);
                chunks.push(chunk);
                offset += chunk_len;
            }
            
            (Bytes::new(), Some(chunks))
        } else {
            // Simple body
            let body_len = header.body_len as usize;
            let body = Bytes::copy_from_slice(&response_slice[offset..offset + body_len]);
            (body, None)
        };
        
        Ok(ResponseData {
            status: header.status,
            headers,
            body: body.0,
            chunks: body.1,
        })
    }
}
```

## Streaming Response Replay

```rust
fn create_streaming_body(&self, chunks: Vec<Bytes>) -> impl Body {
    let (mut tx, body) = hyper::Body::channel();
    
    tokio::spawn(async move {
        for chunk in chunks {
            // Simulate streaming delay (configurable)
            tokio::time::sleep(Duration::from_millis(10)).await;
            
            if let Err(e) = tx.send_data(chunk).await {
                error!("Failed to send chunk: {}", e);
                break;
            }
        }
    });
    
    body
}
```

## WebSocket Replay

```rust
pub async fn replay_websocket(
    &self,
    mut ws: WebSocketStream<TokioIo<Upgraded>>,
    endpoint: EndpointConfig,
) -> Result<()> {
    // Load recorded WebSocket chunks
    let session_id = self.get_session_id_from_websocket(&ws)?;
    let chunks = self.load_websocket_recording(&session_id).await?;
    
    let (mut write, mut read) = ws.split();
    
    for chunk in chunks {
        match chunk.direction {
            Direction::ClientToServer => {
                // Wait for client message
                let client_msg = read.next().await
                    .ok_or(OuliError::WebSocketClosed)??;
                
                // Verify it matches recorded
                let client_data = client_msg.into_data();
                let recorded_data = self.redactor.redact_bytes(&chunk.data);
                
                if client_data != recorded_data {
                    warn!("WebSocket message mismatch");
                    return Err(OuliError::WebSocketMismatch);
                }
            }
            Direction::ServerToClient => {
                // Send recorded server message
                let msg = Message::binary(chunk.data.clone());
                write.send(msg).await?;
            }
        }
    }
    
    Ok(())
}

async fn load_websocket_recording(&self, session_id: &str) -> Result<Vec<WebSocketChunk>> {
    let path = self.storage.websocket_path(session_id);
    let bytes = tokio::fs::read(&path).await?;
    
    // Parse binary WebSocket log
    let mut chunks = Vec::new();
    let mut offset = 0;
    
    while offset < bytes.len() {
        let direction = bytes[offset];
        offset += 1;
        
        let len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;
        
        let data = bytes[offset..offset + len].to_vec();
        offset += len;
        
        chunks.push(WebSocketChunk {
            direction: if direction == 0 {
                Direction::ClientToServer
            } else {
                Direction::ServerToClient
            },
            opcode: 2, // Binary
            data,
            timestamp: 0,
        });
    }
    
    Ok(chunks)
}
```

## Chain Management

```rust
fn get_or_create_chain(&self, session_id: &str) -> MutexGuard<RequestChain> {
    self.chain_tracker
        .entry(session_id.to_string())
        .or_insert_with(RequestChain::new)
        .lock()
        .unwrap()
}

fn reset_chain(&self, session_id: &str) {
    if let Some(mut chain) = self.chain_tracker.get_mut(session_id) {
        chain.reset();
    }
}
```

## Cache Configuration

```rust
impl ReplayCache {
    pub fn new(config: &CacheConfig) -> Self {
        Self {
            recordings: Moka::builder()
                .max_capacity(config.max_recordings)
                .time_to_idle(Duration::from_secs(300)) // 5 min
                .build(),
            
            hot_responses: Moka::builder()
                .max_capacity(config.max_responses)
                .time_to_idle(Duration::from_secs(60)) // 1 min
                .weigher(|_key, value: &Arc<CachedResponse>| {
                    (value.body.len() / 1024) as u32 // Weight by KB
                })
                .build(),
        }
    }
    
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            recordings_cached: self.recordings.entry_count(),
            responses_cached: self.hot_responses.entry_count(),
            total_size_bytes: self.hot_responses.weighted_size() * 1024,
        }
    }
}
```

## Error Responses

```rust
fn recording_not_found_response(request_hash: [u8; 32]) -> Response<BoxBody> {
    let body = serde_json::json!({
        "error": "Recording not found",
        "request_hash": hex::encode(request_hash),
        "hint": "This request was not recorded. Run in record mode first."
    });
    
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("content-type", "application/json")
        .body(BoxBody::new(Full::new(Bytes::from(body.to_string()))))
        .unwrap()
}

fn chain_mismatch_response(expected: [u8; 32], actual: [u8; 32]) -> Response<BoxBody> {
    let body = serde_json::json!({
        "error": "Request chain mismatch",
        "expected_prev_hash": hex::encode(expected),
        "actual_prev_hash": hex::encode(actual),
        "hint": "Request order differs from recording. Reset chain or record again."
    });
    
    Response::builder()
        .status(StatusCode::CONFLICT)
        .header("content-type", "application/json")
        .body(BoxBody::new(Full::new(Bytes::from(body.to_string()))))
        .unwrap()
}
```

## Warmup

```rust
pub async fn warmup(&self, session_ids: &[String]) -> Result<()> {
    for session_id in session_ids {
        // Pre-load recording into cache
        self.get_or_load_recording(session_id).await?;
        
        // Optionally pre-load hot responses
        let reader = self.cache.recordings.get(session_id).unwrap();
        
        for entry in reader.all_entries() {
            let response = reader.read_response(entry)?;
            
            let cached = Arc::new(CachedResponse {
                status: response.status,
                headers: response.headers,
                body: response.body,
                chunks: response.chunks,
            });
            
            self.cache.hot_responses.insert(entry.request_hash, cached);
        }
    }
    
    Ok(())
}
```

## Monitoring

```rust
pub struct ReplayMetrics {
    pub total_requests: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub not_found: AtomicU64,
    pub chain_mismatches: AtomicU64,
    pub avg_latency_us: AtomicU64,
}

impl ReplayEngine {
    pub fn record_request(&self, latency_us: u64, hit: bool) {
        self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);
        
        if hit {
            self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);
        }
        
        // Update rolling average
        let current_avg = self.metrics.avg_latency_us.load(Ordering::Relaxed);
        let new_avg = (current_avg * 99 + latency_us) / 100;
        self.metrics.avg_latency_us.store(new_avg, Ordering::Relaxed);
    }
}
```

## Testing

```rust
#[tokio::test]
async fn test_replay_simple() {
    // First record
    let recording_engine = RecordingEngine::new(test_config()).await.unwrap();
    let request = test_request();
    recording_engine.record_interaction(request, &test_endpoint()).await.unwrap();
    recording_engine.finalize_all().await.unwrap();
    
    // Then replay
    let replay_engine = ReplayEngine::new(test_config()).await.unwrap();
    let request = test_request();
    let response = replay_engine.replay_interaction(request, &test_endpoint()).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_replay_chain() {
    // Record chain
    let recording_engine = RecordingEngine::new(test_config()).await.unwrap();
    
    for i in 0..100 {
        let request = test_request_with_id(i);
        recording_engine.record_interaction(request, &test_endpoint()).await.unwrap();
    }
    
    recording_engine.finalize_all().await.unwrap();
    
    // Replay chain
    let replay_engine = ReplayEngine::new(test_config()).await.unwrap();
    
    for i in 0..100 {
        let request = test_request_with_id(i);
        let response = replay_engine.replay_interaction(request, &test_endpoint()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn test_replay_performance() {
    let replay_engine = ReplayEngine::new(test_config()).await.unwrap();
    
    // Warmup
    replay_engine.warmup(&["test"]).await.unwrap();
    
    let start = Instant::now();
    let iterations = 10000;
    
    for _ in 0..iterations {
        let request = test_request();
        replay_engine.replay_interaction(request, &test_endpoint()).await.unwrap();
    }
    
    let elapsed = start.elapsed();
    let avg_latency = elapsed / iterations;
    
    println!("Average latency: {:?}", avg_latency);
    assert!(avg_latency < Duration::from_micros(100));
}
```

## Performance Targets

| Metric | Target | Strategy |
|--------|--------|----------|
| Replay latency (p50) | < 50 μs | mmap + cache |
| Replay latency (p99) | < 100 μs | Pre-warming |
| Cache hit rate | > 95% | LRU + weight |
| Memory per recording | < 10 MB | Lazy loading |
| Throughput | > 100k req/s | Zero-copy |

## References

- [RFC-002: Binary Storage Format](002-binary-format.md)
- [RFC-003: Request Fingerprinting](003-request-fingerprinting.md)
- [Moka Cache](https://github.com/moka-rs/moka)

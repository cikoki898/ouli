# RFC-005: Recording Engine

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Design the recording engine that captures HTTP/WebSocket traffic and stores it in the binary format defined in RFC-002, with atomic writes and integrity guarantees.

## Architecture

```rust
pub struct RecordingEngine {
    storage: Arc<StorageEngine>,
    redactor: Arc<Redactor>,
    active_recordings: DashMap<String, RecordingSession>,
    config: Arc<Config>,
}

pub struct RecordingSession {
    writer: Mutex<RecordingWriter>,
    chain: Mutex<RequestChain>,
    created_at: Instant,
    interaction_count: AtomicUsize,
}
```

## Request Flow

```rust
impl RecordingEngine {
    pub async fn record_interaction(
        &self,
        request: Request<Incoming>,
        endpoint: &EndpointConfig,
    ) -> Result<Response<BoxBody>> {
        // 1. Extract body (consuming)
        let (parts, body) = request.into_parts();
        let body_bytes = self.read_body(body).await?;
        
        // 2. Redact sensitive data
        let redacted_request = self.redact_request(&parts, &body_bytes)?;
        
        // 3. Compute fingerprint
        let session_id = self.get_session_id(&parts)?;
        let session = self.get_or_create_session(&session_id).await?;
        
        let mut chain = session.chain.lock().await;
        let request_hash = chain.process_request(&redacted_request, &self.redactor);
        
        // 4. Forward to target
        let target_response = self.proxy_to_target(
            &parts,
            body_bytes.clone(),
            endpoint
        ).await?;
        
        // 5. Capture response
        let (response_parts, response_body) = target_response.into_parts();
        let response_bytes = self.read_body(response_body).await?;
        
        // 6. Redact response
        let redacted_response = self.redact_response(
            &response_parts,
            &response_bytes
        )?;
        
        // 7. Store interaction
        let mut writer = session.writer.lock().await;
        writer.append_interaction(
            request_hash,
            chain.previous_hash(),
            &redacted_request,
            &redacted_response,
        )?;
        
        session.interaction_count.fetch_add(1, Ordering::SeqCst);
        
        // 8. Return response to client
        let response = Response::from_parts(
            response_parts,
            BoxBody::new(Full::new(response_bytes))
        );
        
        Ok(response)
    }
}
```

## Body Handling

```rust
async fn read_body(&self, mut body: Incoming) -> Result<Bytes> {
    let mut buf = BytesMut::with_capacity(8192);
    
    while let Some(frame) = body.frame().await {
        let frame = frame?;
        
        if let Some(data) = frame.data_ref() {
            // Check size limit
            if buf.len() + data.len() > MAX_REQUEST_SIZE {
                return Err(OuliError::RequestTooLarge(buf.len() + data.len()));
            }
            
            buf.extend_from_slice(data);
        }
    }
    
    Ok(buf.freeze())
}
```

## Proxying

```rust
async fn proxy_to_target(
    &self,
    parts: &request::Parts,
    body: Bytes,
    endpoint: &EndpointConfig,
) -> Result<Response<Incoming>> {
    let uri = format!(
        "{}://{}:{}{}",
        endpoint.target_type,
        endpoint.target_host,
        endpoint.target_port,
        parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
    );
    
    let mut proxy_req = Request::builder()
        .method(&parts.method)
        .uri(uri)
        .version(parts.version);
    
    // Copy headers (excluding hop-by-hop headers)
    for (name, value) in &parts.headers {
        if !is_hop_by_hop_header(name) {
            proxy_req = proxy_req.header(name, value);
        }
    }
    
    let proxy_req = proxy_req
        .body(Full::new(body))
        .map_err(|e| OuliError::RequestBuildFailed(e.to_string()))?;
    
    // Create client with TLS if needed
    let client = if endpoint.target_type == "https" {
        self.get_https_client()?
    } else {
        self.get_http_client()?
    };
    
    client.request(proxy_req).await
        .map_err(Into::into)
}

fn is_hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection" | "keep-alive" | "proxy-authenticate" |
        "proxy-authorization" | "te" | "trailers" |
        "transfer-encoding" | "upgrade"
    )
}
```

## Session Management

```rust
async fn get_or_create_session(&self, session_id: &str) -> Result<Arc<RecordingSession>> {
    // Check if session exists
    if let Some(session) = self.active_recordings.get(session_id) {
        return Ok(session.clone());
    }
    
    // Create new session
    let path = self.storage.recording_path(session_id);
    let recording_id = self.compute_session_hash(session_id);
    
    let writer = RecordingWriter::create(&path, recording_id)?;
    
    let session = Arc::new(RecordingSession {
        writer: Mutex::new(writer),
        chain: Mutex::new(RequestChain::new()),
        created_at: Instant::now(),
        interaction_count: AtomicUsize::new(0),
    });
    
    self.active_recordings.insert(session_id.to_string(), session.clone());
    
    Ok(session)
}

fn get_session_id(&self, parts: &request::Parts) -> Result<String> {
    // Check for custom test name header
    if let Some(name) = parts.headers.get("x-ouli-test-name") {
        let name_str = name.to_str()
            .map_err(|_| OuliError::InvalidTestName)?;
        
        validate_test_name(name_str)?;
        return Ok(name_str.to_string());
    }
    
    // Fall back to auto-generated name from first request
    // (will be updated once we have the hash)
    Ok("pending".to_string())
}
```

## WebSocket Recording

```rust
pub async fn record_websocket(
    &self,
    mut ws: WebSocketStream<TokioIo<Upgraded>>,
    endpoint: EndpointConfig,
) -> Result<()> {
    // Connect to target
    let target_uri = format!(
        "{}://{}:{}",
        if endpoint.target_type == "https" { "wss" } else { "ws" },
        endpoint.target_host,
        endpoint.target_port
    );
    
    let (target_ws, _) = tokio_tungstenite::connect_async(target_uri).await?;
    let (mut target_write, mut target_read) = target_ws.split();
    let (mut client_write, mut client_read) = ws.split();
    
    // Create recording buffer
    let mut chunks = Vec::new();
    
    // Bidirectional forwarding with recording
    let client_to_server = async {
        while let Some(msg) = client_read.next().await {
            let msg = msg?;
            
            // Record: client -> server
            chunks.push(WebSocketChunk {
                direction: Direction::ClientToServer,
                opcode: msg.opcode(),
                data: msg.into_data(),
                timestamp: now_ns(),
            });
            
            // Forward to target
            target_write.send(msg).await?;
        }
        Ok::<_, OuliError>(())
    };
    
    let server_to_client = async {
        while let Some(msg) = target_read.next().await {
            let msg = msg?;
            
            // Record: server -> client
            chunks.push(WebSocketChunk {
                direction: Direction::ServerToClient,
                opcode: msg.opcode(),
                data: msg.into_data(),
                timestamp: now_ns(),
            });
            
            // Forward to client
            client_write.send(msg).await?;
        }
        Ok::<_, OuliError>(())
    };
    
    // Run both directions concurrently
    tokio::try_join!(client_to_server, server_to_client)?;
    
    // Store recorded chunks
    self.store_websocket_recording(&chunks).await?;
    
    Ok(())
}
```

## Streaming Responses (SSE)

```rust
async fn record_streaming_response(
    &self,
    mut response_body: Incoming,
) -> Result<(Vec<Bytes>, Bytes)> {
    let mut chunks = Vec::new();
    let mut complete = BytesMut::new();
    
    while let Some(frame) = response_body.frame().await {
        let frame = frame?;
        
        if let Some(data) = frame.data_ref() {
            // Check size limit
            if complete.len() + data.len() > MAX_RESPONSE_SIZE {
                return Err(OuliError::ResponseTooLarge(complete.len() + data.len()));
            }
            
            // Store chunk
            chunks.push(data.clone());
            complete.extend_from_slice(data);
        }
    }
    
    Ok((chunks, complete.freeze()))
}
```

## Finalization

```rust
pub async fn finalize_session(&self, session_id: &str) -> Result<()> {
    let session = self.active_recordings.remove(session_id)
        .ok_or(OuliError::SessionNotFound)?
        .1;
    
    let writer = session.writer.into_inner();
    writer.finalize()?;
    
    let duration = session.created_at.elapsed();
    let count = session.interaction_count.load(Ordering::SeqCst);
    
    info!(
        "Finalized recording '{}': {} interactions in {:?}",
        session_id, count, duration
    );
    
    Ok(())
}

pub async fn finalize_all(&self) -> Result<()> {
    let sessions: Vec<_> = self.active_recordings.iter()
        .map(|entry| entry.key().clone())
        .collect();
    
    for session_id in sessions {
        self.finalize_session(&session_id).await?;
    }
    
    Ok(())
}
```

## Error Recovery

```rust
impl RecordingSession {
    async fn checkpoint(&self) -> Result<()> {
        // Flush current state to disk
        let writer = self.writer.lock().await;
        writer.flush()?;
        Ok(())
    }
    
    async fn recover(&self, last_good_offset: u64) -> Result<()> {
        // Truncate file to last good state
        let writer = self.writer.lock().await;
        writer.truncate(last_good_offset)?;
        
        // Reset chain to last known hash
        let mut chain = self.chain.lock().await;
        chain.rewind_to(last_good_offset)?;
        
        Ok(())
    }
}
```

## Atomic Writes

```rust
impl RecordingWriter {
    pub fn append_interaction_atomic(
        &mut self,
        request_hash: [u8; 32],
        prev_hash: [u8; 32],
        request: &RequestData,
        response: &ResponseData,
    ) -> Result<()> {
        // Write to temporary buffer first
        let mut temp_buf = Vec::new();
        self.write_interaction_to_buffer(
            &mut temp_buf,
            request_hash,
            prev_hash,
            request,
            response,
        )?;
        
        // Verify buffer integrity
        let checksum = crc32(&temp_buf);
        
        // Atomic write to mmap
        let offset = self.data_offset;
        self.mmap[offset..offset + temp_buf.len()]
            .copy_from_slice(&temp_buf);
        
        // Update metadata only after successful write
        self.data_offset += temp_buf.len() as u64;
        self.header.interaction_count += 1;
        
        Ok(())
    }
}
```

## Monitoring

```rust
pub struct RecordingMetrics {
    pub total_requests: AtomicU64,
    pub total_bytes_recorded: AtomicU64,
    pub active_sessions: AtomicUsize,
    pub errors: AtomicU64,
}

impl RecordingEngine {
    pub fn metrics(&self) -> RecordingMetrics {
        RecordingMetrics {
            total_requests: self.total_requests.load(Ordering::Relaxed).into(),
            total_bytes_recorded: self.total_bytes.load(Ordering::Relaxed).into(),
            active_sessions: self.active_recordings.len().into(),
            errors: self.errors.load(Ordering::Relaxed).into(),
        }
    }
}
```

## Testing

```rust
#[tokio::test]
async fn test_record_simple_request() {
    let engine = RecordingEngine::new(test_config()).await.unwrap();
    
    let request = Request::builder()
        .method("GET")
        .uri("/api/test")
        .header("x-ouli-test-name", "simple_get")
        .body(Full::new(Bytes::new()))
        .unwrap();
    
    let response = engine.record_interaction(request, &test_endpoint()).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify recording file exists
    let path = engine.storage.recording_path("simple_get");
    assert!(path.exists());
    
    // Verify can be read back
    let reader = RecordingReader::open(&path).unwrap();
    assert_eq!(reader.interaction_count(), 1);
}

#[tokio::test]
async fn test_record_chain() {
    let engine = RecordingEngine::new(test_config()).await.unwrap();
    
    for i in 0..10 {
        let request = Request::builder()
            .method("POST")
            .uri(format!("/api/test/{}", i))
            .header("x-ouli-test-name", "chain_test")
            .body(Full::new(Bytes::from(format!("request {}", i))))
            .unwrap();
        
        engine.record_interaction(request, &test_endpoint()).await.unwrap();
    }
    
    engine.finalize_session("chain_test").await.unwrap();
    
    // Verify chain is intact
    let reader = RecordingReader::open(
        &engine.storage.recording_path("chain_test")
    ).unwrap();
    
    assert_eq!(reader.interaction_count(), 10);
    
    // Verify chain linkage
    let interactions = reader.all_interactions();
    for i in 1..interactions.len() {
        assert_eq!(
            interactions[i].prev_request_hash,
            interactions[i-1].request_hash
        );
    }
}
```

## Performance Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| Record overhead | < 500 μs | Per request |
| Proxy latency | < 1 ms | Network dependent |
| Write throughput | > 10k req/s | Sustained |
| Memory per session | < 1 MB | Bounded |

## References

- [RFC-002: Binary Storage Format](002-binary-format.md)
- [RFC-003: Request Fingerprinting](003-request-fingerprinting.md)
- [RFC-007: Security and Redaction](007-security-redaction.md)

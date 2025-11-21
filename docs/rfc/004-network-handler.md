# RFC-004: Network Protocol Handler

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Design the async network layer using Tokio for handling HTTP/1.1, HTTP/2, and WebSocket connections with bounded concurrency and zero-copy I/O.

## Motivation

Requirements:
- Handle 4,096 concurrent connections
- Support HTTP/1.1, HTTP/2, WebSocket
- Backpressure and flow control
- Graceful shutdown
- Connection pooling with bounded memory

## Architecture

```rust
pub struct NetworkHandler {
    config: Arc<Config>,
    connection_pool: ConnectionPool,
    recording_store: Arc<RecordingStore>,
    shutdown: broadcast::Sender<()>,
}

impl NetworkHandler {
    pub async fn run(self) -> Result<()> {
        let mut shutdown_rx = self.shutdown.subscribe();
        
        // Start all endpoint listeners
        let mut tasks = JoinSet::new();
        
        for endpoint in &self.config.endpoints {
            let handler = self.clone();
            let endpoint = endpoint.clone();
            
            tasks.spawn(async move {
                handler.run_endpoint(endpoint).await
            });
        }
        
        // Wait for shutdown signal or task failure
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received");
            }
            Some(result) = tasks.join_next() => {
                if let Err(e) = result {
                    error!("Endpoint task failed: {}", e);
                }
            }
        }
        
        // Graceful shutdown
        tasks.shutdown().await;
        Ok(())
    }
    
    async fn run_endpoint(&self, endpoint: EndpointConfig) -> Result<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], endpoint.source_port as u16));
        let listener = TcpListener::bind(addr).await?;
        
        info!("Listening on {}", addr);
        
        loop {
            let (stream, peer_addr) = listener.accept().await?;
            
            // Check connection limit
            if !self.connection_pool.can_accept() {
                warn!("Connection limit reached, rejecting {}", peer_addr);
                drop(stream);
                continue;
            }
            
            let handler = self.clone();
            let endpoint = endpoint.clone();
            
            tokio::spawn(async move {
                if let Err(e) = handler.handle_connection(stream, endpoint).await {
                    error!("Connection error: {}", e);
                }
            });
        }
    }
}
```

## Connection Pool

```rust
pub struct ConnectionPool {
    active: Arc<AtomicUsize>,
    limit: usize,
    arenas: Mutex<Vec<Arena>>,
}

impl ConnectionPool {
    pub fn new(limit: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            limit,
            arenas: Mutex::new(Vec::new()),
        }
    }
    
    pub fn can_accept(&self) -> bool {
        self.active.load(Ordering::Relaxed) < self.limit
    }
    
    pub fn acquire(&self) -> Option<ConnectionGuard> {
        let current = self.active.fetch_add(1, Ordering::SeqCst);
        
        if current >= self.limit {
            self.active.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        
        Some(ConnectionGuard {
            active: self.active.clone(),
            arena: self.allocate_arena(),
        })
    }
    
    fn allocate_arena(&self) -> Arena {
        let mut arenas = self.arenas.lock().unwrap();
        
        // Reuse existing arena if available
        if let Some(arena) = arenas.pop() {
            arena.reset();
            arena
        } else {
            Arena::new()
        }
    }
    
    fn return_arena(&self, arena: Arena) {
        let mut arenas = self.arenas.lock().unwrap();
        arenas.push(arena);
    }
}

pub struct ConnectionGuard {
    active: Arc<AtomicUsize>,
    arena: Arena,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        // Return arena to pool
    }
}
```

## HTTP Handler

```rust
async fn handle_connection(
    &self,
    stream: TcpStream,
    endpoint: EndpointConfig,
) -> Result<()> {
    let guard = self.connection_pool.acquire()
        .ok_or(OuliError::ConnectionLimitReached)?;
    
    match endpoint.source_type.as_str() {
        "http" => self.handle_http(stream, endpoint, guard).await,
        "https" => self.handle_https(stream, endpoint, guard).await,
        _ => Err(OuliError::InvalidProtocol),
    }
}

async fn handle_http(
    &self,
    stream: TcpStream,
    endpoint: EndpointConfig,
    _guard: ConnectionGuard,
) -> Result<()> {
    let io = TokioIo::new(stream);
    
    let service = service_fn(|req: Request<Incoming>| {
        let handler = self.clone();
        let endpoint = endpoint.clone();
        async move {
            handler.handle_request(req, endpoint).await
        }
    });
    
    if let Err(e) = http1::Builder::new()
        .serve_connection(io, service)
        .await
    {
        error!("HTTP connection error: {}", e);
    }
    
    Ok(())
}

async fn handle_request(
    &self,
    request: Request<Incoming>,
    endpoint: EndpointConfig,
) -> Result<Response<BoxBody>> {
    // Check for WebSocket upgrade
    if hyper_tungstenite::is_upgrade_request(&request) {
        return self.handle_websocket_upgrade(request, endpoint).await;
    }
    
    match self.config.mode {
        Mode::Record => self.record_request(request, endpoint).await,
        Mode::Replay => self.replay_request(request, endpoint).await,
    }
}
```

## WebSocket Handler

```rust
async fn handle_websocket_upgrade(
    &self,
    mut request: Request<Incoming>,
    endpoint: EndpointConfig,
) -> Result<Response<BoxBody>> {
    let (response, websocket) = hyper_tungstenite::upgrade(&mut request, None)?;
    
    let handler = self.clone();
    
    tokio::spawn(async move {
        match websocket.await {
            Ok(ws) => {
                if let Err(e) = handler.handle_websocket(ws, endpoint).await {
                    error!("WebSocket error: {}", e);
                }
            }
            Err(e) => error!("WebSocket upgrade error: {}", e),
        }
    });
    
    Ok(response.map(BoxBody::new))
}

async fn handle_websocket(
    &self,
    ws: WebSocketStream<TokioIo<Upgraded>>,
    endpoint: EndpointConfig,
) -> Result<()> {
    match self.config.mode {
        Mode::Record => self.record_websocket(ws, endpoint).await,
        Mode::Replay => self.replay_websocket(ws, endpoint).await,
    }
}
```

## Backpressure

```rust
pub struct BackpressureConfig {
    pub max_in_flight: usize,
    pub buffer_size: usize,
    pub timeout: Duration,
}

pub struct BackpressureController {
    semaphore: Arc<Semaphore>,
    timeout: Duration,
}

impl BackpressureController {
    pub async fn acquire(&self) -> Result<SemaphorePermit> {
        tokio::time::timeout(
            self.timeout,
            self.semaphore.acquire()
        )
        .await
        .map_err(|_| OuliError::Timeout)?
        .map_err(Into::into)
    }
}
```

## Streaming Support

```rust
async fn stream_response_body(
    mut body: Incoming,
    mut tx: mpsc::Sender<Bytes>,
) -> Result<()> {
    while let Some(chunk) = body.frame().await {
        let frame = chunk?;
        
        if let Some(data) = frame.data_ref() {
            tx.send(data.clone()).await
                .map_err(|_| OuliError::ChannelClosed)?;
        }
    }
    
    Ok(())
}
```

## Graceful Shutdown

```rust
pub async fn shutdown_gracefully(&self, timeout: Duration) -> Result<()> {
    // Signal all tasks to stop accepting new connections
    let _ = self.shutdown.send(());
    
    // Wait for active connections to complete
    let deadline = Instant::now() + timeout;
    
    while self.connection_pool.active.load(Ordering::Relaxed) > 0 {
        if Instant::now() > deadline {
            warn!("Shutdown timeout, force closing connections");
            break;
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    Ok(())
}
```

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Connection limit reached: {0}")]
    ConnectionLimitReached(usize),
    
    #[error("Invalid protocol: {0}")]
    InvalidProtocol(String),
    
    #[error("WebSocket upgrade failed: {0}")]
    WebSocketUpgradeFailed(String),
    
    #[error("Request timeout after {}s", .0.as_secs())]
    Timeout(Duration),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),
}
```

## Performance Optimizations

### Zero-Copy Body Forwarding

```rust
async fn forward_body(
    source: &mut Incoming,
    target: &mut SendRequest<BoxBody>,
) -> Result<()> {
    // Use hyper's body streaming without copying to intermediate buffer
    let response = target.send_request(request).await?;
    Ok(())
}
```

### Connection Reuse

```rust
pub struct ConnectionCache {
    pool: deadpool::managed::Pool<HttpConnection>,
}

impl ConnectionCache {
    async fn get_connection(&self, target: &str) -> Result<HttpConnection> {
        self.pool.get().await.map_err(Into::into)
    }
}
```

### TCP Tuning

```rust
fn configure_tcp_socket(socket: &TcpSocket) -> Result<()> {
    socket.set_nodelay(true)?; // Disable Nagle's algorithm
    socket.set_recv_buffer_size(256 * 1024)?; // 256 KB
    socket.set_send_buffer_size(256 * 1024)?; // 256 KB
    Ok(())
}
```

## Testing

```rust
#[tokio::test]
async fn test_concurrent_connections() {
    let handler = NetworkHandler::new(test_config()).await.unwrap();
    
    let mut tasks = vec![];
    
    for i in 0..1000 {
        tasks.push(tokio::spawn(async move {
            let client = reqwest::Client::new();
            client.get(format!("http://localhost:8080/test{}", i))
                .send()
                .await
                .unwrap()
        }));
    }
    
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::test]
async fn test_graceful_shutdown() {
    let handler = NetworkHandler::new(test_config()).await.unwrap();
    
    // Start some long-running connections
    let mut tasks = vec![];
    for _ in 0..10 {
        tasks.push(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }));
    }
    
    // Trigger shutdown
    handler.shutdown_gracefully(Duration::from_secs(10)).await.unwrap();
    
    // Verify all connections completed
    assert_eq!(handler.connection_pool.active.load(Ordering::Relaxed), 0);
}
```

## Benchmarks

Target performance:

| Metric | Target | Measurement |
|--------|--------|-------------|
| Connections/sec | 10,000 | wrk benchmark |
| Request latency p50 | < 1 ms | Histogram |
| Request latency p99 | < 10 ms | Histogram |
| Memory per connection | < 32 KB | jemalloc stats |
| Connection setup | < 100 μs | Custom timer |

## References

- [Hyper Documentation](https://hyper.rs/)
- [Tokio Guide](https://tokio.rs/tokio/tutorial)
- [HTTP/2 RFC 7540](https://www.rfc-editor.org/rfc/rfc7540)
- [WebSocket RFC 6455](https://www.rfc-editor.org/rfc/rfc6455)

# RFC-008: Performance Optimization

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Define comprehensive performance optimization strategies to achieve sub-100μs replay latency and > 100k req/s throughput while maintaining safety guarantees.

## Performance Targets

| Metric | Target | Current (Go) | Improvement |
|--------|--------|--------------|-------------|
| Replay p50 latency | < 50 μs | ~500 μs | 10× |
| Replay p99 latency | < 100 μs | ~2 ms | 20× |
| Record throughput | 100k req/s | 20k req/s | 5× |
| Memory/connection | < 32 KB | ~128 KB | 4× |
| Binary size | < 5 MB | 15 MB | 3× |
| Cold start | < 50 ms | ~200 ms | 4× |

## Memory-Mapped I/O

### Zero-Copy Reads

```rust
pub struct MmapReader {
    mmap: Mmap,
    _phantom: PhantomData<&'static [u8]>,
}

impl MmapReader {
    pub fn read_slice(&self, offset: u64, len: usize) -> &[u8] {
        // Direct memory access - no copying
        &self.mmap[offset as usize..(offset as usize + len)]
    }
    
    pub fn read_bytes(&self, offset: u64, len: usize) -> Bytes {
        // Bytes created from mmap slice with Arc refcount
        // No data copying until write
        Bytes::copy_from_slice(self.read_slice(offset, len))
    }
}
```

### Huge Pages

```rust
pub fn enable_transparent_hugepages(mmap: &Mmap) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use libc::{madvise, MADV_HUGEPAGE};
        
        unsafe {
            let result = madvise(
                mmap.as_ptr() as *mut _,
                mmap.len(),
                MADV_HUGEPAGE
            );
            
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    
    Ok(())
}
```

## Connection Pooling

### Object Pool

```rust
pub struct ConnectionPool<T> {
    pool: crossbeam::queue::ArrayQueue<T>,
    factory: Arc<dyn Fn() -> T + Send + Sync>,
    stats: PoolStats,
}

impl<T> ConnectionPool<T> {
    pub fn acquire(&self) -> PooledObject<T> {
        let obj = self.pool.pop().unwrap_or_else(|| {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
            (self.factory)()
        });
        
        self.stats.hits.fetch_add(1, Ordering::Relaxed);
        
        PooledObject {
            obj: Some(obj),
            pool: self.pool.clone(),
        }
    }
}

pub struct PooledObject<T> {
    obj: Option<T>,
    pool: Arc<crossbeam::queue::ArrayQueue<T>>,
}

impl<T> Drop for PooledObject<T> {
    fn drop(&mut self) {
        if let Some(obj) = self.obj.take() {
            let _ = self.pool.push(obj);
        }
    }
}
```

### Arena Allocation

```rust
pub struct Arena {
    bump: bumpalo::Bump,
    size: AtomicUsize,
}

impl Arena {
    pub fn new() -> Self {
        Self {
            bump: bumpalo::Bump::with_capacity(64 * 1024), // 64 KB
            size: AtomicUsize::new(0),
        }
    }
    
    pub fn alloc<T>(&self, value: T) -> &mut T {
        self.size.fetch_add(std::mem::size_of::<T>(), Ordering::Relaxed);
        self.bump.alloc(value)
    }
    
    pub fn reset(&mut self) {
        self.bump.reset();
        self.size.store(0, Ordering::Relaxed);
    }
    
    pub fn size_bytes(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }
}
```

## HTTP Client Pooling

```rust
pub struct HttpClientPool {
    pool: deadpool::managed::Pool<HttpClient>,
}

impl HttpClientPool {
    pub async fn get(&self, target: &str) -> Result<HttpClient> {
        // Reuse existing connection if available
        self.pool.get().await.map_err(Into::into)
    }
}

pub struct HttpClient {
    client: hyper::Client<HttpsConnector<HttpConnector>>,
    keep_alive: bool,
}

impl HttpClient {
    pub fn new() -> Self {
        let https = hyper_rustls::HttpsConnectorBuilder::new()
            .with_native_roots()
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();
        
        let client = hyper::Client::builder()
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .build(https);
        
        Self {
            client,
            keep_alive: true,
        }
    }
}
```

## Lock-Free Data Structures

### Request Chain Tracker

```rust
pub struct LockFreeChainTracker {
    chains: DashMap<String, AtomicU64>,
    hashes: DashMap<u64, [u8; 32]>,
}

impl LockFreeChainTracker {
    pub fn next_hash(&self, session_id: &str, request_hash: [u8; 32]) -> u64 {
        let counter = self.chains
            .entry(session_id.to_string())
            .or_insert(AtomicU64::new(0));
        
        let seq = counter.fetch_add(1, Ordering::SeqCst);
        self.hashes.insert(seq, request_hash);
        
        seq
    }
}
```

## Async I/O Optimization

### io_uring (Linux)

```rust
#[cfg(target_os = "linux")]
pub struct IoUringDriver {
    ring: io_uring::IoUring,
}

impl IoUringDriver {
    pub async fn read_file(&self, fd: RawFd, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        
        let entry = io_uring::opcode::Read::new(
            io_uring::types::Fd(fd),
            buf.as_mut_ptr(),
            len as _
        )
        .offset(offset)
        .build();
        
        unsafe {
            self.ring.submission()
                .push(&entry)
                .map_err(|_| OuliError::IoUringFull)?;
        }
        
        self.ring.submit_and_wait(1)?;
        
        let cqe = self.ring.completion().next()
            .ok_or(OuliError::IoUringTimeout)?;
        
        if cqe.result() < 0 {
            return Err(OuliError::IoError(cqe.result()));
        }
        
        Ok(buf)
    }
}
```

## CPU Optimization

### SIMD for Redaction

```rust
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

pub fn redact_simd(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    #[target_feature(enable = "avx2")]
    unsafe fn redact_avx2(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
        // Use AVX2 for parallel comparison
        // 32 bytes at a time
        let mut result = Vec::with_capacity(haystack.len());
        
        // ... AVX2 implementation ...
        
        result
    }
    
    #[cfg(target_feature = "avx2")]
    unsafe { redact_avx2(haystack, needle, replacement) }
    
    #[cfg(not(target_feature = "avx2"))]
    redact_scalar(haystack, needle, replacement)
}
```

### Branch Prediction Hints

```rust
#[inline(always)]
pub fn likely(b: bool) -> bool {
    #[cold]
    fn cold() {}
    
    if !b { cold() }
    b
}

#[inline(always)]
pub fn unlikely(b: bool) -> bool {
    #[cold]
    fn cold() {}
    
    if b { cold() }
    b
}

// Usage
pub fn lookup_response(&self, hash: [u8; 32]) -> Option<Response> {
    if likely(self.cache.contains(&hash)) {
        // Fast path: cache hit
        self.cache.get(&hash)
    } else {
        // Slow path: load from disk
        self.load_from_disk(hash)
    }
}
```

## Cache Optimization

### L1/L2 Cache Alignment

```rust
#[repr(C, align(64))] // Cache line size
pub struct CacheAlignedEntry {
    pub hash: [u8; 32],
    pub offset: u64,
    pub size: u32,
    _padding: [u8; 20], // Pad to 64 bytes
}

static_assertions::const_assert_eq!(
    std::mem::size_of::<CacheAlignedEntry>(),
    64
);
```

### Prefetching

```rust
pub fn prefetch_entries(&self, hashes: &[[u8; 32]]) {
    for hash in hashes {
        if let Some(entry) = self.index.get(hash) {
            // Prefetch data into cache
            unsafe {
                std::intrinsics::prefetch_read_data(
                    &self.mmap[entry.offset as usize] as *const _,
                    3 // Locality hint
                );
            }
        }
    }
}
```

## Serialization Optimization

### Zero-Copy Deserialization

```rust
use zerocopy::{AsBytes, FromBytes};

#[derive(AsBytes, FromBytes)]
#[repr(C)]
pub struct RecordingHeader {
    magic: [u8; 8],
    version: u32,
    // ...
}

impl RecordingHeader {
    pub fn from_bytes(bytes: &[u8]) -> Option<&Self> {
        // Zero-copy cast - no deserialization overhead
        zerocopy::LayoutVerified::<_, Self>::new(bytes)
            .map(|lv| lv.into_ref())
    }
}
```

## Benchmarking

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn bench_fingerprint(c: &mut Criterion) {
    let mut group = c.benchmark_group("fingerprint");
    
    for size in [100, 1_000, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &size,
            |b, &size| {
                let request = create_test_request(size);
                let redactor = Redactor::new(&RedactionConfig::default()).unwrap();
                
                b.iter(|| {
                    fingerprint_request(
                        black_box(&request),
                        black_box(CHAIN_HEAD_HASH),
                        black_box(&redactor)
                    )
                });
            }
        );
    }
    
    group.finish();
}

fn bench_replay_lookup(c: &mut Criterion) {
    let reader = setup_test_recording();
    let hash = test_request_hash();
    
    c.bench_function("replay_lookup", |b| {
        b.iter(|| {
            reader.lookup(black_box(hash))
        });
    });
}

criterion_group!(benches, bench_fingerprint, bench_replay_lookup);
criterion_main!(benches);
```

## Profiling

### CPU Profiling

```rust
#[cfg(feature = "profiling")]
pub fn profile_section<F, R>(name: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let guard = pprof::ProfilerGuard::new(100).unwrap();
    let result = f();
    
    if let Ok(report) = guard.report().build() {
        let file = std::fs::File::create(format!("{}.pb", name)).unwrap();
        report.pprof().unwrap().write_to_writer(&mut BufWriter::new(file)).unwrap();
    }
    
    result
}
```

### Memory Profiling

```rust
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

pub fn print_memory_stats() {
    let allocated = jemalloc_ctl::stats::allocated::read().unwrap();
    let resident = jemalloc_ctl::stats::resident::read().unwrap();
    
    println!("Memory allocated: {} MB", allocated / 1024 / 1024);
    println!("Memory resident: {} MB", resident / 1024 / 1024);
}
```

## Compile-Time Optimization

### LTO (Link-Time Optimization)

```toml
[profile.release]
lto = "fat"           # Full LTO
codegen-units = 1     # Better optimization
opt-level = 3         # Maximum optimization
panic = "abort"       # Smaller binary
strip = true          # Remove debug symbols
```

### PGO (Profile-Guided Optimization)

```bash
# Step 1: Build with instrumentation
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build --release

# Step 2: Run benchmarks
./target/release/ouli benchmark

# Step 3: Merge profile data
llvm-profdata merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data

# Step 4: Build with optimization
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata" cargo build --release
```

## Performance Testing

```rust
#[tokio::test]
async fn test_replay_latency_p99() {
    let engine = ReplayEngine::new(test_config()).await.unwrap();
    engine.warmup(&["test"]).await.unwrap();
    
    let mut latencies = Vec::new();
    
    for _ in 0..10_000 {
        let start = Instant::now();
        let _ = engine.replay_interaction(test_request(), &test_endpoint()).await.unwrap();
        latencies.push(start.elapsed().as_micros());
    }
    
    latencies.sort_unstable();
    
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[latencies.len() * 99 / 100];
    
    println!("p50: {}μs, p99: {}μs", p50, p99);
    
    assert!(p50 < 50, "p50 latency too high: {}μs", p50);
    assert!(p99 < 100, "p99 latency too high: {}μs", p99);
}

#[tokio::test]
async fn test_throughput() {
    let engine = ReplayEngine::new(test_config()).await.unwrap();
    
    let start = Instant::now();
    let iterations = 100_000;
    
    for _ in 0..iterations {
        let _ = engine.replay_interaction(test_request(), &test_endpoint()).await.unwrap();
    }
    
    let elapsed = start.elapsed();
    let throughput = iterations as f64 / elapsed.as_secs_f64();
    
    println!("Throughput: {:.0} req/s", throughput);
    
    assert!(throughput > 100_000.0, "Throughput too low: {:.0}", throughput);
}
```

## Monitoring

```rust
pub struct PerformanceMetrics {
    pub latency_histogram: Histogram,
    pub throughput_counter: Counter,
    pub memory_gauge: Gauge,
}

impl PerformanceMetrics {
    pub fn record_request(&self, latency_us: u64) {
        self.latency_histogram.observe(latency_us as f64);
        self.throughput_counter.inc();
    }
    
    pub fn report(&self) -> PerformanceReport {
        PerformanceReport {
            p50: self.latency_histogram.percentile(0.50),
            p99: self.latency_histogram.percentile(0.99),
            p999: self.latency_histogram.percentile(0.999),
            throughput: self.throughput_counter.rate(),
            memory_mb: self.memory_gauge.value() / 1024.0 / 1024.0,
        }
    }
}
```

## References

- [Tokio Performance Tuning](https://tokio.rs/tokio/topics/performance)
- [Linux Performance](https://www.brendangregg.com/linuxperf.html)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [io_uring](https://kernel.dk/io_uring.pdf)

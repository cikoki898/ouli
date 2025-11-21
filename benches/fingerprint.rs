use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use ouli::fingerprint::{fingerprint_request, Request, CHAIN_HEAD_HASH};

fn bench_fingerprint_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("fingerprint");

    for size in [100, 1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            let request = Request {
                method: "POST".to_string(),
                path: "/api/test".to_string(),
                query: vec![],
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: vec![b'x'; size],
            };

            b.iter(|| fingerprint_request(black_box(&request), black_box(CHAIN_HEAD_HASH)));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_fingerprint_sizes);
criterion_main!(benches);

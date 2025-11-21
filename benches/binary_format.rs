use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ouli::storage::{RecordingReader, RecordingWriter};
use tempfile::NamedTempFile;

fn bench_write_performance(c: &mut Criterion) {
    c.bench_function("write_100_interactions", |b| {
        b.iter(|| {
            let file = NamedTempFile::new().unwrap();
            let recording_id = [0u8; 32];
            let mut writer = RecordingWriter::create(file.path(), recording_id).unwrap();

            for i in 0..100u8 {
                let request_hash = [i; 32];
                let prev_hash = if i == 0 { [0u8; 32] } else { [i - 1; 32] };
                let request_data = b"GET /api/test HTTP/1.1\r\nHost: example.com\r\n\r\n";
                let response_data = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello";

                writer
                    .append_interaction(
                        black_box(request_hash),
                        black_box(prev_hash),
                        black_box(request_data),
                        black_box(response_data),
                    )
                    .unwrap();
            }

            writer.finalize().unwrap();
        });
    });
}

fn bench_read_performance(c: &mut Criterion) {
    let file = NamedTempFile::new().unwrap();
    let recording_id = [0u8; 32];

    // Setup: Write 100 interactions
    {
        let mut writer = RecordingWriter::create(file.path(), recording_id).unwrap();
        for i in 0..100u8 {
            let request_hash = [i; 32];
            let prev_hash = if i == 0 { [0u8; 32] } else { [i - 1; 32] };
            writer
                .append_interaction(
                    request_hash,
                    prev_hash,
                    b"GET /test HTTP/1.1\r\n\r\n",
                    b"HTTP/1.1 200 OK\r\n\r\n",
                )
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    c.bench_function("lookup_interaction", |b| {
        let reader = RecordingReader::open(file.path()).unwrap();

        b.iter(|| {
            let request_hash = [50u8; 32];
            let entry = reader.lookup(black_box(request_hash)).unwrap();
            let _request = reader.read_request(&entry).unwrap();
            let _response = reader.read_response(&entry).unwrap();
        });
    });
}

criterion_group!(benches, bench_write_performance, bench_read_performance);
criterion_main!(benches);

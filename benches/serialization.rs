// benches/serialization.rs
//
// Criterion benchmarks for streaming serialization.
//
// These benchmarks measure the throughput of the JSON/CSV row encoders and
// the BatchSizer adaptation speed using synthetic data (no database required).
//
// Run with:
//   cargo bench --bench serialization

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use pgmcp::streaming::BatchSizer;
use pgmcp::streaming::csv::write_csv_field;
use pgmcp::streaming::json::write_json_string;

// ── Benchmark: write_json_string throughput ───────────────────────────────────

fn bench_json_string_short(c: &mut Criterion) {
    let s = "hello world";
    c.bench_function("json_string_short", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(32);
            write_json_string(s, &mut buf);
            buf
        });
    });
}

fn bench_json_string_medium(c: &mut Criterion) {
    let s = "This is a medium length string with some content that is representative of a typical column value in a PostgreSQL row.";
    c.bench_function("json_string_medium", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(256);
            write_json_string(s, &mut buf);
            buf
        });
    });
}

fn bench_json_string_with_escaping(c: &mut Criterion) {
    // String with many characters that need escaping.
    let s = r#"He said "hello\nworld" and she replied "goodbye\tthere""#;
    c.bench_function("json_string_with_escaping", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(128);
            write_json_string(s, &mut buf);
            buf
        });
    });
}

// ── Benchmark: write_csv_field throughput ────────────────────────────────────

fn bench_csv_field_plain(c: &mut Criterion) {
    let s = "hello world 12345";
    c.bench_function("csv_field_plain", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(32);
            write_csv_field(s, &mut buf);
            buf
        });
    });
}

fn bench_csv_field_quoted(c: &mut Criterion) {
    let s = r#"value with "quotes" and, commas"#;
    c.bench_function("csv_field_quoted", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(64);
            write_csv_field(s, &mut buf);
            buf
        });
    });
}

// ── Benchmark: BatchSizer adaptation ─────────────────────────────────────────

fn bench_batch_sizer_adaptation(c: &mut Criterion) {
    // Simulate a query producing variable-size rows across 20 batches.
    // Each batch has a different byte count to exercise the adaptation logic.
    c.bench_function("batch_sizer_adaptation_20_batches", |b| {
        b.iter(|| {
            let mut sizer = BatchSizer::new();
            let mut total_rows = 0usize;
            for batch_idx in 0..20usize {
                let batch_size = sizer.next_batch_size();
                // Simulate variable row sizes.
                let bytes_per_row = 64 + (batch_idx * 32);
                let bytes = batch_size * bytes_per_row;
                sizer.record(batch_size, bytes);
                total_rows += batch_size;
            }
            total_rows
        });
    });
}

// ── Benchmark: Simulated row encoding (no DB) ─────────────────────────────────

fn bench_json_encode_1000_rows_synthetic(c: &mut Criterion) {
    // Encode 1000 synthetic key-value pairs as JSON objects using the low-level
    // string writer. This approximates the throughput of the hot encoding path
    // for text-heavy result sets.
    let values: Vec<String> = (0..1000).map(|i| format!("value_number_{i:05}")).collect();

    c.bench_function("json_encode_1000_synthetic_text_rows", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(values.len() * 40);
            buf.push(b'[');
            let mut first = true;
            for (i, v) in values.iter().enumerate() {
                if !first {
                    buf.push(b',');
                }
                first = false;
                buf.extend_from_slice(b"{");
                write_json_string("id", &mut buf);
                buf.push(b':');
                buf.extend_from_slice(i.to_string().as_bytes());
                buf.push(b',');
                write_json_string("value", &mut buf);
                buf.push(b':');
                write_json_string(v, &mut buf);
                buf.push(b'}');
            }
            buf.push(b']');
            buf
        });
    });
}

fn bench_csv_encode_1000_rows_synthetic(c: &mut Criterion) {
    let values: Vec<String> = (0..1000).map(|i| format!("value_number_{i:05}")).collect();

    c.bench_function("csv_encode_1000_synthetic_rows", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(values.len() * 30);
            // Header.
            buf.extend_from_slice(b"id,value\r\n");
            for (i, v) in values.iter().enumerate() {
                buf.extend_from_slice(i.to_string().as_bytes());
                buf.push(b',');
                write_csv_field(v, &mut buf);
                buf.extend_from_slice(b"\r\n");
            }
            buf
        });
    });
}

// ── Parameterized: varying row widths ─────────────────────────────────────────

fn bench_json_varying_row_width(c: &mut Criterion) {
    let widths = [16usize, 64, 256, 1024];
    let mut group = c.benchmark_group("json_string_by_width");
    for &width in &widths {
        let s: String = "x".repeat(width);
        group.bench_with_input(BenchmarkId::from_parameter(width), &s, |b, s| {
            b.iter(|| {
                let mut buf = Vec::with_capacity(width + 4);
                write_json_string(s, &mut buf);
                buf
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_json_string_short,
    bench_json_string_medium,
    bench_json_string_with_escaping,
    bench_csv_field_plain,
    bench_csv_field_quoted,
    bench_batch_sizer_adaptation,
    bench_json_encode_1000_rows_synthetic,
    bench_csv_encode_1000_rows_synthetic,
    bench_json_varying_row_width,
);
criterion_main!(benches);

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use masa::Context;
use std::time::Duration;
use tonic::metadata::MetadataMap;

fn bench_serialization(c: &mut Criterion) {
    let ctx = masa::create_context("test-api", Duration::from_micros(100));
    let ctx_str = ctx.to_header_string();

    let mut group = c.benchmark_group("Serialization");

    group.bench_function("to_header_string", |b| {
        b.iter(|| black_box(&ctx).to_header_string())
    });

    group.bench_function("from_header_string", |b| {
        b.iter(|| Context::from_header_string(black_box(&ctx_str)))
    });

    group.finish();
}

fn bench_metadata_map(c: &mut Criterion) {
    let ctx = masa::create_context("test-api", Duration::from_micros(100));

    let mut group = c.benchmark_group("MetadataMap");

    group.bench_function("insert_ctx", |b| {
        b.iter_batched(
            || MetadataMap::new(),
            |mut map| {
                map.insert_ctx("x-masa-context", &ctx);
                map
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.bench_function("get_ctx", |b| {
        let mut map = MetadataMap::new();
        map.insert_ctx("x-masa-context", &ctx);
        b.iter(|| map.get_ctx(black_box("x-masa-context")))
    });

    group.finish();
}

criterion_group!(benches, bench_serialization, bench_metadata_map);
criterion_main!(benches);

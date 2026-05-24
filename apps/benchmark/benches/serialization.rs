use criterion::{black_box, criterion_group, criterion_main, Criterion};
use masa::{get_masa_context_from_metadata, set_masa_context_in_metadata, Context};
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

    group.bench_function("set_masa_context_in_metadata", |b| {
        b.iter_batched(
            || MetadataMap::new(),
            |mut map| {
                set_masa_context_in_metadata(&mut map, &ctx);
                map
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.bench_function("get_masa_context_from_metadata", |b| {
        let mut map = MetadataMap::new();
        set_masa_context_in_metadata(&mut map, &ctx);
        b.iter(|| get_masa_context_from_metadata(black_box(&map)))
    });

    group.finish();
}

criterion_group!(benches, bench_serialization, bench_metadata_map);
criterion_main!(benches);

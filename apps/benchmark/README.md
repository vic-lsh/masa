# Masa Benchmarks

This crate contains benchmarks for measuring the performance of the Masa RPC framework, specifically focusing on metadata serialization overhead and end-to-end (E2E) request latency.

## Prerequisites

Ensure you have the Rust toolchain installed.

## Running Benchmarks

### 1. Statistical Benchmarks (Criterion)

For rigorous, statistically significant performance measurements, use `cargo bench`. This uses [Criterion.rs](https://github.com/bheisler/criterion.rs) to automatically handle warm-up, multiple sampling, and outlier detection.

```bash
cargo bench -p masa-benchmark
```

This will run two suites:
*   **`serialization`**: Microbenchmarks for `Context` serialization (`to_header_string`), deserialization (`from_header_string`), and `MetadataMap` insertion/extraction.
*   **`e2e`**: End-to-end latency measurement of a gRPC "ping" request with Masa context headers attached, reusing a client connection.

**Output:**
Results will be printed to the console, and detailed HTML reports (including graphs) will be generated in `target/criterion/report/index.html`.

### 2. Ad-hoc/Simple Benchmark

For a quick, single-pass check without statistical analysis, you can run the binary directly. This runs a simple loop and prints averages and a histogram for E2E latency.

```bash
cargo run -p masa-benchmark --release
```

**Output:**
Prints the average time per operation for serialization tasks and latency percentiles (Min, P50, P95, P99, Max) for the E2E test.

## Benchmark Details

*   **Serialization:** Measures the cost of converting a `masa::Context` struct into a Base64-encoded Bincode string and back.
*   **MetadataMap:** Measures the overhead of inserting and retrieving the context from Tonic's `MetadataMap`.
*   **E2E:** Sets up a local gRPC server and client (using the `frontend.proto` definition) and measures the round-trip time (RTT) of a `HandlePing` RPC call carrying Masa metadata.

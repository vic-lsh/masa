# syntax=docker/dockerfile:1.5

FROM rust:1.90.0-bookworm AS builder

WORKDIR /app

# Install protobuf-compiler and other essential tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    libprotobuf-dev \
    build-essential \
    pkg-config \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN rustup override set nightly

# Copy workspace sources needed for a full build
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY 3rd_party 3rd_party
COPY apps apps
COPY libs libs

ARG FEATURE_ARG=fifo

# Build all binaries once so both targets can reuse the output
RUN cargo build --release --bin generic-service --features "${FEATURE_ARG}"
RUN cargo build --release --bin mssim-loadgen

COPY trace-analysis trace-analysis
RUN ln -s /app/trace-analysis /trace-analysis

FROM debian:trixie-slim AS generic-service
COPY --from=builder /app/target/release/generic-service /usr/local/bin/generic-service

WORKDIR /app
ENV RUST_BACKTRACE=1
CMD ["/usr/local/bin/generic-service"]

FROM debian:trixie-slim AS loadgen
COPY --from=builder /app/target/release/mssim-loadgen /usr/local/bin/mssim-loadgen
COPY --from=builder /app/trace-analysis /app/trace-analysis
COPY --from=builder /trace-analysis /trace-analysis

WORKDIR /app
ENV RUST_BACKTRACE=1
CMD ["/usr/local/bin/mssim-loadgen"]

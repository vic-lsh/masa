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
COPY apps apps
COPY libs libs

ARG CACHE_ID=mssim

# Build all binaries once so both targets can reuse the output
# Use BuildKit cache mounts for faster rebuilds (requires BuildKit/buildx).
# Keep the target directory in the image so runtime stages can copy the binaries.
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry-${CACHE_ID},sharing=locked \
    cargo build --release --bin mssim-loadgen
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry-${CACHE_ID},sharing=locked \
    cargo build --release --bin generic-service

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

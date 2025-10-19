# Use a recent Rust version for building
FROM rust:latest AS builder

ARG FEATURE_ARG=fifo
ENV FEATURE=${FEATURE_ARG}

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

# We need to copy the entire project (from workspace root)
# This is because we want to link the generic-service with our runtime,
# as well as other crates (e.g., sim-config)
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY 3rd_party 3rd_party
COPY apps apps
COPY libs libs

# Build the generic service
RUN cargo build --release --bin generic-service --features ${FEATURE_ARG}

FROM debian:trixie-slim
COPY --from=builder /app/target/release/generic-service /usr/local/bin/generic-service

WORKDIR /app

ENV RUST_BACKTRACE=1
CMD ["/usr/local/bin/generic-service"]

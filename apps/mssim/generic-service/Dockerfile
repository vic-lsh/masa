# Use a recent Rust version for building
FROM rust:latest AS rust_builder

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
RUN cargo build --release --bin generic-service --features prio_global

# Declare the build argument for the container port (providing a default value of 8000)
ARG SERVICE_CONTAINER_PORT=8000 

# Expose the gRPC port using the build argument
EXPOSE ${SERVICE_CONTAINER_PORT}

# Command to run the service
# The service code should read the port from the SERVICE_PORT environment variable
ENV RUST_BACKTRACE=1
CMD ["cargo", "run", "--release", "--bin", "generic-service", "--features", "prio_global"]
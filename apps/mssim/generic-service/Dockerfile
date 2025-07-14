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

# Copy manifest files
COPY Cargo.toml Cargo.lock ./

# Create dummy source file for initial fetch
RUN mkdir src && echo 'fn main() {}' > src/main.rs

# Fetch dependencies
RUN cargo fetch

# Copy the actual source code
COPY src ./src
COPY proto ./proto 
COPY build.rs .

# Build the generic service
RUN cargo build --release

# Declare the build argument for the container port (providing a default value of 8000)
ARG SERVICE_CONTAINER_PORT=8000 

# Expose the gRPC port using the build argument
EXPOSE ${SERVICE_CONTAINER_PORT}

# Command to run the service
# The service code should read the port from the SERVICE_PORT environment variable
CMD ["cargo", "run", "--release"]

# Stage 1: Build the application
FROM rustlang/rust:nightly AS rust_builder

# Install build-time dependencies, including the protobuf compiler
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the entire project workspace into the build stage
COPY . .

# Build only the specific service binary needed for this image
ARG SERVICE_NAME
RUN cargo build --release --bin ${SERVICE_NAME} -j 2

# Stage 2: Create the final, minimal image for deployment
# FROM debian:bullseye-slim
FROM debian:sid-slim


# Install only the required runtime dependencies
# RUN apt-get update && apt-get install -y --no-install-recommends \
#     libssl1.1 \
#     ca-certificates \
#     && rm -rf /var/lib/apt/lists/*
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*


# Copy the compiled binary from the builder stage
ARG SERVICE_NAME
COPY --from=rust_builder /app/target/release/${SERVICE_NAME} /usr/local/bin/service

CMD ["/usr/local/bin/service"]

# # Stage 1: Build the application
# FROM rustlang/rust:nightly AS rust_builder

# WORKDIR /app

# # Copy the top-level manifest files that define the whole workspace
# COPY Cargo.toml Cargo.lock ./

# # Copy all the application and library source code
# COPY apps ./apps
# COPY libs ./libs
# COPY 3rd_party ./3rd_party

# # Build dependencies for the entire workspace to leverage caching
# RUN cargo build --workspace --release

# # Build the specific service binary we need
# ARG SERVICE_NAME
# RUN cargo build --release -p ${SERVICE_NAME}


# # Stage 2: Create the final, minimal image
# FROM debian:bullseye-slim

# RUN apt-get update && apt-get install -y --no-install-recommends \
#     libssl1.1 ca-certificates && rm -rf /var/lib/apt/lists/*

# # Copy the compiled binary from the builder stage
# ARG SERVICE_NAME
# COPY --from=rust_builder /app/target/release/${SERVICE_NAME} /usr/local/bin/service

# CMD ["/usr/local/bin/service"]
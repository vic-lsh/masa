# Stage 1: Build the application
FROM rustlang/rust:nightly AS rust_builder
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler pkg-config libssl-dev build-essential \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy all necessary source folders from the project root
COPY apps ./apps
COPY libs ./libs
COPY 3rd_party ./3rd_party
COPY apps/socialnet/proto ./apps/socialnet/proto

# Build the entire workspace
RUN cargo build --release -p socialnet

# Stage 2: Create the final, minimal image
FROM debian:sid-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app

# Copy all compiled binaries
COPY --from=rust_builder /app/target/release/ /usr/local/bin/

# Set up the entrypoint script (ensure entrypoint.sh is also in the socialnet folder)
COPY apps/socialnet/socialnet_entrypoint.sh /usr/local/bin/socialnet_entrypoint.sh
RUN chmod +x /usr/local/bin/socialnet_entrypoint.sh
ENTRYPOINT [ "sh", "-c" ]
CMD ["/usr/local/bin/socialnet_entrypoint.sh"]

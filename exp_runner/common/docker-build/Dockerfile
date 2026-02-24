# Stage 1: Build all binaries for the app
FROM rustlang/rust:nightly-slim AS builder

ARG APP
ARG FEATURES

# Combine update+install to avoid stale package indexes from cached layers.
RUN apt-get update && apt-get install -y\
  curl\
  unzip\
  protobuf-compiler\
  pkg-config\
  libssl-dev

# Create a new empty shell project
WORKDIR /usr/src/masa

# Copy source code
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY apps apps
COPY libs libs

# Build all binaries for the app once
# Use BuildKit cache mounts for faster rebuilds (requires BuildKit/buildx). Keep
# the target directory in the image so runtime stage can copy the binaries.
# Use a unique cache ID per build to avoid race conditions in parallel builds.
ARG CACHE_ID=${APP}
WORKDIR /usr/src/masa/apps/${APP}
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry-${CACHE_ID},sharing=locked \
  bash -c "if [ -z \"$FEATURES\" ]; then \
  cargo build -p ${APP} --bins --release; \
  else \
  cargo build -p ${APP} --bins --release --features ${FEATURES}; \
  fi"

# Stage 2: Base runtime image with dependencies
FROM debian:trixie-slim AS runtime-base

ARG APP
ARG FEATURES
ARG LOG_LEVEL=info
ARG GEN_CONFIG_PATH
ARG CACHE_ID=${APP}

# Combine update+install to avoid stale package indexes from cached layers.
RUN apt-get update && apt-get install -y\
  libssl-dev

# Copy entrypoint script
COPY exp_runner/common/docker-build/entrypoint.sh /usr/entrypoint.sh

# Copy gen config - GEN_CONFIG_PATH must be provided as build arg
COPY ${GEN_CONFIG_PATH} /usr/gen_config.json

ENV LOG_LEVEL=${LOG_LEVEL}

# Stage 3: Runtime image with a specific binary
FROM runtime-base AS runtime

ARG FEATURES
ARG BINARY_NAME
ARG CACHE_ID=${APP}

COPY --from=builder --chmod=755 /usr/src/masa/target/release/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

ENTRYPOINT [ "sh", "-c" ]
CMD ["/usr/entrypoint.sh"]

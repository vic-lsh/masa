# Runtime-only Dockerfile — used when the builder image already exists locally.
# Avoids resolving rustlang/rust:nightly-slim from Docker Hub.
# BUILDER_IMAGE must be passed as --build-arg and refer to a local image.

ARG BUILDER_IMAGE=hotel_builder:latest

# Reference the pre-built builder so COPY --from can use it below
FROM ${BUILDER_IMAGE} AS prebuilt_builder

# Stage 1: Base runtime image with OS dependencies
FROM debian:trixie-slim AS runtime-base

ARG APP
ARG FEATURES
ARG LOG_LEVEL=info
ARG GEN_CONFIG_PATH
ARG CACHE_ID=${APP}

RUN apt-get update && apt-get install -y \
  libssl-dev

COPY exp_runner/common/docker-build/entrypoint.sh /usr/entrypoint.sh
COPY ${GEN_CONFIG_PATH} /usr/gen_config.json

ENV LOG_LEVEL=${LOG_LEVEL}

# Stage 2: Runtime image with a specific binary copied from the pre-built builder
FROM runtime-base AS runtime

ARG FEATURES
ARG BINARY_NAME
ARG CACHE_ID=${APP}

COPY --from=prebuilt_builder --chmod=755 /usr/src/masa/target/release/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

ENTRYPOINT [ "sh", "-c" ]
CMD ["/usr/entrypoint.sh"]


FROM rustlang/rust:nightly-slim AS builder

ARG APP
ARG FEATURES
ARG BINARIES=""

RUN apt-get update
RUN apt-get install -y\
  curl\
  unzip\
  protobuf-compiler\
  pkg-config\
  libssl-dev

# Create a new empty shell project
WORKDIR /usr/src/masa

# build every service in the app
# Note: we build every service once here. They're cached then reused when we
# generate per-service image in the second stage below.
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY 3rd_party 3rd_party
COPY apps apps
COPY libs libs

WORKDIR /usr/src/masa/apps/${APP}
RUN if [ -z "$FEATURES" ]; then\
  cargo build -p ${APP} --bins --release;\
  else \
  cargo build -p ${APP} --bins --release --features ${FEATURES};\
  fi

# Copy the binary exporting script
COPY exp/common/docker-build/export-binaries.sh /tmp/export-binaries.sh
RUN chmod +x /tmp/export-binaries.sh

# Copy specified binaries to a temporary directory for the final stage
RUN BINARIES="$BINARIES" RELEASE_DIR=/usr/src/masa/target/release OUTPUT_DIR=/tmp/binaries /tmp/export-binaries.sh

# Final stage - create a slimmer image
FROM debian:trixie-slim

ARG APP
ARG LOG_LEVEL=warn
ARG APP_CONFIG_PATH
ARG GEN_CONFIG_PATH

RUN apt-get update

# re-install libssl-dev, because it is a runtime dependency
# we choose to re-install rather than copy the installed .so file from the
# prior stage, because we could miss copying some other deps that ssl needs.
RUN apt-get install -y\
  libssl-dev

# Copy the binaries directory from the builder stage
COPY --from=builder /tmp/binaries/ /usr/local/bin/

COPY exp/common/docker-build/entrypoint.sh /usr/entrypoint.sh
# Copy app config - APP_CONFIG_PATH must be provided as build arg
# If config is optional for an app, a default/empty config should be provided
COPY ${APP_CONFIG_PATH} /usr/config.json
# Copy gen config - GEN_CONFIG_PATH must be provided as build arg
COPY ${GEN_CONFIG_PATH} /usr/gen_config.json

ENV LOG_LEVEL=${LOG_LEVEL}

ENTRYPOINT [ "sh", "-c" ]
CMD ["/usr/entrypoint.sh"]

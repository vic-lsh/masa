
FROM rustlang/rust:nightly-slim AS builder

ARG APP
ARG FEATURES

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
COPY . .
WORKDIR /usr/src/masa/apps/${APP}
RUN if [ -z "$FEATURES" ]; then\
  cargo build -p ${APP} --bins --release;\
else \
  cargo build -p ${APP} --bins --release --features ${FEATURES};\
fi

# Final stage - create a slimmer image
FROM debian:trixie-slim

ARG APP
ARG LOG_LEVEL=warn
ARG BINARY_NAME
RUN if [ -z "$BINARY_NAME" ]; then \
    echo "Error: BINARY_NAME argument is required" && exit 1; \
    fi

RUN apt-get update

# re-install libssl-dev, because it is a runtime dependency
# we choose to re-install rather than copy the installed .so file from the
# prior stage, because we could miss copying some other deps that ssl needs.
RUN apt-get install -y\
        libssl-dev

# Copy the built executable from the builder stage
COPY --from=builder /usr/src/masa/target/release/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}
COPY apps/${APP}/scripts/local/config.docker.json /usr/config.json

ENV BINARY_NAME=${BINARY_NAME}
ENV LOG_LEVEL=${LOG_LEVEL}

ENTRYPOINT [ "sh", "-c" ]
CMD ["RUST_LOG=${LOG_LEVEL} /usr/local/bin/${BINARY_NAME} --config /usr/config.json"]

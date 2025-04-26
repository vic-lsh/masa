FROM rustlang/rust:nightly-slim AS builder

ARG FEATURES

RUN apt-get update
RUN apt-get install -y\
        curl\
        unzip\
        protobuf-compiler\
        pkg-config\
        libssl-dev

# Create a new empty shell project
WORKDIR /usr/src/app
RUN cargo init

# Copy over dependencies and libs.
# Order these from least to most likely to be modified.
COPY Cargo.toml Cargo.lock* ./
COPY 3rd_party ./3rd_party
COPY libs ./libs
COPY apps/app-util-macros/ ./apps/app-util-macros/
COPY apps/app-utils/ ./apps/app-utils/

COPY apps/hotel/Cargo.toml ./apps/hotel/Cargo.toml
COPY apps/socialnet/Cargo.toml ./apps/socialnet/Cargo.toml

WORKDIR /usr/src/app/apps/hotel
RUN mkdir src
RUN touch src/lib.rs

# Build the dependencies first. This will be cached.
RUN if [ -z "$FEATURES" ]; then\
  cargo build --release --lib;\
else \
  cargo build --release --lib --features ${FEATURES};\
fi
WORKDIR /usr/src/app/

# build every service in hotel
# Note: we build every service once here. They're cached then reused when we
# generate per-service image in the second stage below.
COPY apps ./apps
WORKDIR /usr/src/app/apps/hotel
RUN if [ -z "$FEATURES" ]; then\
  cargo build --release;\
else \
  cargo build --release --features ${FEATURES};\
fi

# Final stage - create a slimmer image
FROM debian:bookworm-slim

ARG LOG_LEVEL=warn
ARG BINARY_NAME
ARG HOTEL_CONFIG
RUN if [ -z "$BINARY_NAME" ]; then \
    echo "Error: BINARY_NAME argument is required" && exit 1; \
    fi
RUN if [ -z "$HOTEL_CONFIG" ]; then \
    echo "Error: HOTEL_CONFIG argument is required" && exit 1; \
    fi

RUN apt-get update

# re-install libssl-dev, because it is a runtime dependency
# we choose to re-install rather than copy the installed .so file from the
# prior stage, because we could miss copying some other deps that ssl needs.
RUN apt-get install -y\
        libssl-dev

# Copy the built executable from the builder stage
COPY --from=builder /usr/src/app/target/release/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}
COPY ${HOTEL_CONFIG} /usr/hotel_config.json

ENV BINARY_NAME=${BINARY_NAME}
ENV HOTEL_CONFIG=${HOTEL_CONFIG}
ENV LOG_LEVEL=${LOG_LEVEL}

ENTRYPOINT [ "sh", "-c" ]
CMD ["RUST_LOG=${LOG_LEVEL} /usr/local/bin/${BINARY_NAME} --config /usr/hotel_config.json"]

FROM rustlang/rust:nightly AS builder
ARG FEATURES

RUN apt-get update
RUN apt-get install -y\
        curl\
        unzip\
        protobuf-compiler\
        pkg-config\
        libssl-dev

WORKDIR /usr/src/app
RUN cargo init

# Copy over dependencies and libs.
# Order these from least to most likely to be modified.
COPY Cargo.toml Cargo.lock* ./
COPY 3rd_party ./3rd_party
COPY libs ./libs

COPY apps/socialnet/Cargo.toml ./apps/socialnet/Cargo.toml
COPY apps/hotel/Cargo.toml ./apps/hotel/Cargo.toml

WORKDIR /usr/src/app/apps/socialnet
RUN mkdir src
RUN touch src/lib.rs

RUN if [ -z "$FEATURES" ]; then\
  cargo build --release --lib;\
else \
  cargo build --release --lib --features ${FEATURES};\
fi
WORKDIR /usr/src/app/

COPY apps ./apps
WORKDIR /usr/src/app/apps/socialnet

# build every service in socialnet
RUN if [ -z "$FEATURES" ]; then\
  cargo build --release;\
else \
  cargo build --release --features ${FEATURES};\
fi

FROM debian:bookworm-slim

ARG LOG_LEVEL=warn
ARG BINARY_NAME
RUN if [ -z "$BINARY_NAME" ]; then \
    echo "Error: BINARY_NAME argument is required" && exit 1; \
    fi

RUN apt-get update

RUN apt-get install -y\ 
        libssl-dev

COPY --from=builder /usr/src/app/target/release/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}

ENV BINARY_NAME=${BINARY_NAME}
ENV LOG_LEVEL=${LOG_LEVEL}

CMD sh -c "RUST_LOG=$LOG_LEVEL exec /usr/local/bin/$BINARY_NAME"
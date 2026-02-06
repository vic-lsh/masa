FROM rustlang/rust:nightly

# Install system dependencies
# protobuf-compiler: for tonic/prost
# docker.io: for dind
# libgraphviz-dev, pkg-config: for pygraphviz
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    docker.io \
    libgraphviz-dev \
    pkg-config \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install uv
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
ENV PATH="/root/.local/bin:${PATH}"

# Add any other common tools here

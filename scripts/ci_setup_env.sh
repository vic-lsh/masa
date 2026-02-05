#!/bin/bash
set -euo pipefail

# This script sets up the common CI environment dependencies for Masa tests.
# It is intended to be sourced or run in the before_script section of GitLab CI jobs.

echo "Setting up CI environment dependencies..."

# Update apt package index
apt-get update -yqq

# Install system dependencies
# - protobuf-compiler: Required for Rust tonic_build (gRPC)
# - docker.io: Required for Docker-in-Docker (dind) interactions
# - libgraphviz-dev, pkg-config: Required for pygraphviz (Python dependency for plotting)
# - curl, ca-certificates: Required for downloading tools (uv, rustup, etc.)
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    protobuf-compiler \
    docker.io \
    libgraphviz-dev \
    pkg-config \
    curl \
    ca-certificates

# Install uv (Python package manager) if not present
if ! command -v uv &> /dev/null; then
    echo "Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    # Add uv to PATH for the current session
    export PATH="$HOME/.local/bin:$PATH"
else
    echo "uv is already installed."
fi

echo "CI environment setup complete."
if command -v protoc &> /dev/null; then echo "protoc: $(protoc --version)"; fi
if command -v uv &> /dev/null; then echo "uv: $(uv --version)"; fi

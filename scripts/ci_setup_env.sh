#!/bin/bash
set -euo pipefail

# This script sets up the common CI environment dependencies for Masa tests.
# It is intended to be sourced or run in the before_script section of GitLab CI jobs.

echo "Setting up CI environment dependencies..."

# Update apt package index
apt-get update -yqq

# Install system dependencies
# - protobuf-compiler: Required for Rust tonic_build (gRPC)
# - libgraphviz-dev, pkg-config: Required for pygraphviz (Python dependency for plotting)
# - curl, ca-certificates: Required for downloading tools (uv, rustup, etc.)
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    protobuf-compiler \
    libgraphviz-dev \
    pkg-config \
    curl \
    ca-certificates

# Install Docker client if not present (required for dind interaction)
if ! command -v docker &> /dev/null; then
    echo "Installing Docker client..."
    DOCKER_VERSION="27.3.1"
    curl -fsSL "https://download.docker.com/linux/static/stable/x86_64/docker-${DOCKER_VERSION}.tgz" -o docker.tgz
    # Extract only the client binary
    tar xzf docker.tgz docker/docker
    mv docker/docker /usr/local/bin/docker
    rm -rf docker docker.tgz
    chmod +x /usr/local/bin/docker
else
    echo "Docker client is already installed."
fi

# Install Docker Buildx plugin
# Check if buildx is working (it might be installed but not in PATH or not as plugin)
if ! docker buildx version &> /dev/null; then
    echo "Installing Docker Buildx..."
    BUILDX_VERSION="v0.31.1"
    # Create the cli-plugins directory for the current user (likely root in CI)
    mkdir -p "$HOME/.docker/cli-plugins"
    curl -fsSL "https://github.com/docker/buildx/releases/download/${BUILDX_VERSION}/buildx-${BUILDX_VERSION}.linux-amd64" -o "$HOME/.docker/cli-plugins/docker-buildx"
    chmod +x "$HOME/.docker/cli-plugins/docker-buildx"
else
    echo "Docker Buildx is already installed."
fi

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
if command -v docker &> /dev/null; then echo "docker: $(docker --version)"; fi

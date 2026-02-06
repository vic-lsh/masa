#!/bin/bash
set -euo pipefail

# Install protoc manually because apt version is too old (v3 vs v25+)
# See: https://protobuf.dev/installation/

PROTOC_VERSION="29.3" # Pin to a recent stable version
ARCH="x86_64"
OS="linux"

echo "Installing protoc v${PROTOC_VERSION}..."

# Ensure we have unzip and curl
if ! command -v unzip &> /dev/null || ! command -v curl &> /dev/null; then
    echo "Installing dependencies (unzip, curl)..."
    if command -v apt-get &> /dev/null; then
        apt-get update -yqq
        apt-get install -y --no-install-recommends unzip curl
    else
        echo "Warning: apt-get not found, assuming dependencies are present or manual install needed."
    fi
fi

# Download
URL="https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-${OS}-${ARCH}.zip"
echo "Downloading $URL..."
curl -LO "${URL}"

# Install
mkdir -p "$HOME/.local"
unzip -o "protoc-${PROTOC_VERSION}-${OS}-${ARCH}.zip" -d "$HOME/.local"
rm "protoc-${PROTOC_VERSION}-${OS}-${ARCH}.zip"

# Add to PATH if not already present
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    export PATH="$HOME/.local/bin:$PATH"
    echo "Temporarily added $HOME/.local/bin to PATH for this script."
fi

echo "protoc installed to $(which protoc)"
protoc --version

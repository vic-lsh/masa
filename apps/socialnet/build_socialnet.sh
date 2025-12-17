#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# go to project root
cd "$SCRIPT_DIR/../.."

echo "Building the generic socialnet service image..."

docker build \
  -t socialnet-generic-svc:latest \
  -f ./apps/socialnet/Dockerfile \
  .

echo "Build complete. Image 'socialnet-generic-svc:latest' is ready."

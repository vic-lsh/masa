#!/bin/bash
set -e

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
    echo "Error: plese run in the apps/hotel directory" >&2
    exit 1
fi

binary=""
features=""
rust_log="warn"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --binary)
        binary="$2"
        shift 2
        ;;
    --features)
        features="$2"
        shift 2
        ;;
    --rust-log)
        rust_log="$2"
        shift 2
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

# Validate required arguments
if [[ -z "$binary" ]]; then
    echo "Error: --binary argument is required"
    exit 1
fi

# go to project root. Dockerfile needs context from project root.
cd ../..

echo "Building docker image for service $binary. features: '$features'."
docker build -f ./apps/hotel/Dockerfile \
    --build-arg FEATURES=$features \
    --build-arg BINARY_NAME=$binary \
    --build-arg LOG_LEVEL=$rust_log \
    --build-arg HOTEL_CONFIG=./apps/hotel/scripts/local/hotel_config.json \
    -t $binary \
    .

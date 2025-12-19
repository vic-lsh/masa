#!/bin/bash

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

features=""
rust_log="warn"
no_cache=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --features)
        features="$2"
        shift 2
        ;;
    --rust-log)
        rust_log="$2"
        shift 2
        ;;
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

# Validate required arguments
if [[ -z "$features" ]]; then
    echo "Warn: --features not set."
fi

app="hotel"

# List of binaries to copy into the image
binaries="hotel_client_bench hotel_frontend hotel_geo hotel_rate hotel_review hotel_search hotel_profile hotel_reservation hotel_user hotel_recommendation loadgen"

echo "Building generic hotel image with all binaries. Feature flags: $features. No cache: '$no_cache'."

# Get the repo root (assuming this script is in exp/common/scripts)
repo_root=$(git rev-parse --show-toplevel)
app_dir="$repo_root/apps/$app"

if [[ ! -d "$app_dir" ]]; then
    echo "Error: application directory '$app_dir' not found" >&2
    exit 1
fi

# Change to app directory (required by Dockerfile context)
cd "$app_dir"

# Go to project root for Docker build context
cd "$repo_root"

if [[ -z "$features" ]]; then
    features_arg=""
else
    features_arg="--build-arg FEATURES=$features"
fi

echo "Building docker image: hotel:latest"
docker build -f ./apps/scripts/Dockerfile \
    $features_arg \
    --build-arg LOG_LEVEL=$rust_log \
    --build-arg APP=$app \
    --build-arg APP_CONFIG_FILE=hotel.json \
    --build-arg BINARIES="$binaries" \
    --ulimit nofile=4096:4096 \
    $no_cache \
    -t hotel:latest \
    .

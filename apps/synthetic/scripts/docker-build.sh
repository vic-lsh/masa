#!/bin/bash

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
COMMON_BUILD_SCRIPT="$SCRIPT_DIR/../../exp/common/scripts/docker-build-svc.sh"

services=(
    "synthetic_frontend"
    "synthetic_child"
)

pwd=$(pwd)
if [[ "$pwd" != */apps/synthetic ]]; then
    echo "Error: please run in the apps/synthetic directory" >&2
    exit 1
fi

features=""
rust_log="warn"

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
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

app="synthetic"

# Validate required arguments
if [[ -z "$features" ]]; then
    echo "Warn: --features not set."
    features_arg=""
else
    features_arg="--features $features"
fi

echo "Building all services. Feature flags: $features."

echo "Building docker images sequentially."

set -e
for svc in "${services[@]}"; do
    "$COMMON_BUILD_SCRIPT" --binary "$svc" --app "$app" --rust-log "$rust_log" $features_arg
done

#!/bin/bash

services=(
    "hotel_frontend"
    "hotel_geo"
    "hotel_rate"
    "hotel_search"
    "hotel_profile"
    "hotel_reservation"
    "hotel_user"
    "hotel_review"
)

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
    echo "Error: plese run in the apps/hotel directory" >&2
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

# Validate required arguments
if [[ -z "$features" ]]; then
    echo "Warn: --features not set."
fi

echo "Building all hotel services. Feature flags: $features."

echo "Building docker images sequentially."

set -e
for svc in "${services[@]}"; do
    if [[ -z "$features" ]]; then
        ./scripts/docker-build-svc.sh --binary $svc --rust-log $rust_log
    else
        ./scripts/docker-build-svc.sh --binary $svc --rust-log $rust_log --features $features
    fi
done

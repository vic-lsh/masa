#!/bin/bash

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

services=(
    "hotel_client_bench"
    "hotel_frontend"
    "hotel_geo"
    "hotel_rate"
    "hotel_search"
    "hotel_profile"
    "hotel_reservation"
    "hotel_user"
    "hotel_review"
)

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

app="hotel"

echo "Building all hotel services. Feature flags: $features."

echo "Building docker images sequentially."

if [[ -z "$features" ]]; then
    features_arg=""
else
    features_arg="--features $features"
fi

set -e
for svc in "${services[@]}"; do
    $SCRIPT_DIR/../common/scripts/docker-build-svc.sh --binary "$svc" --app "$app" --rust-log "$rust_log" --app-config hotel.json $features_arg
done

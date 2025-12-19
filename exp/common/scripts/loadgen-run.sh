#!/bin/bash

set -e

save_logs_arg=""
output_path=
app="${MASA_APP_NAME:-}"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --output)
        output_path=$2
        shift 2
        ;;
    --save-logs)
        save_logs_arg="--save-logs"
        shift 1
        ;;
    --app)
        app=$2
        shift 2
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

if [[ -z "$output_path" ]]; then
    echo "--output must be set"
    exit 1
fi

if [[ -z "$app" ]]; then
    echo "Error: unable to determine app for loadgen run. Pass --app or set MASA_APP_NAME." >&2
    exit 1
fi

case "$app" in
    hotel)
        container_name="hotel_client_bench"
        network="local_hotel_network"
        image_name="hotel:latest"
        binary_name="hotel_client_bench"
        ;;
    synthetic)
        container_name="synthetic_client_bench"
        network="local_synthetic_network"
        image_name="synthetic_client_bench"
        binary_name="synthetic_client_bench"
        ;;
    *)
        echo "Error: unsupported app '$app' for load generator" >&2
        exit 1
        ;;
esac

docker rm -f "$container_name" >/dev/null 2>&1 || true

mkdir -p "$output_path"

# Make sure that this is consistent with the network name created by docker compose
docker run \
    --name "$container_name" \
    --network "$network" \
    -e BINARY_NAME="$binary_name" \
    -e LOG_LEVEL="${LOG_LEVEL:-warn}" \
    "$image_name" \
    &> "$output_path/loadgen.log"

# this should be hard-coded in the docker entrypoint.sh
container_trace_path="/tmp/masa-load-gen"

# Copy traces from inside the container to outside
docker cp "$container_name:$container_trace_path" "$output_path"
# hacky way to make sure that the trace files are actually placed in $output_path
if [[ -d "$output_path/masa-load-gen" ]]; then
    shopt -s nullglob
    for trace_file in "$output_path/masa-load-gen"/*; do
        mv "$trace_file" "$output_path/"
    done
    shopt -u nullglob
    rm -rf "$output_path/masa-load-gen"
fi

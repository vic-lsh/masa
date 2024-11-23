#!/bin/bash
set -e

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

binary=""
features=""

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

echo "Building service $binary. Feature flags: $features."

cmd="cargo build --release --bin $binary"
output=$(eval "$cmd" 2>&1)
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Build failed with exit code $exit_code"
    echo "Output:"
    echo "$output"
    exit $exit_code
fi

mkdir -p tmp
cp ../target/release/$binary tmp/

echo "Building docker image for service $binary."
docker build -f Dockerfile.template \
    --build-arg BINARY_PATH=tmp \
    --build-arg BINARY_NAME=$binary \
    --build-arg CONFIG_PATH=./snippets/search_reservation \
    --build-arg CONFIG_NAME=hotel_config.json \
    -t $binary .

#!/bin/bash
set -e

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
    echo "Error: plese run in the apps/hotel directory" >&2
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

mkdir -p tmp
cp ../../target/release/$binary tmp/

echo "Building docker image for service $binary."
docker build -f Dockerfile.template \
    --build-arg BINARY_PATH=tmp \
    --build-arg BINARY_NAME=$binary \
    --build-arg SNIPPET_PATH=./snippets/search-reservation \
    --build-arg HOTEL_CONFIG=hotel_config.json \
    -t $binary .

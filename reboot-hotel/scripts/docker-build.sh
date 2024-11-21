#!/bin/bash
set -e

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

if [ $# -ne 1 ]; then
    echo "Error: argument incorrect."
    echo "Usage: $0 <binary-name>"
    echo "<binary-name> should match those listed in Cargo.toml."
    exit 1
fi

binary=$1

mkdir -p tmp
cp ../target/release/$binary tmp/

docker build -f Dockerfile.template \
    --build-arg BINARY_PATH=tmp \
    --build-arg BINARY_NAME=$binary \
    --build-arg CONFIG_PATH=./snippets/search_reservation \
    --build-arg CONFIG_NAME=hotel_config.json \
    -t $binary .

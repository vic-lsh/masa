#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

cargo run --release --bin hotel_client_bench -- \
    --gen-config $SCRIPT_DIR/gen_config.json \
    --output-path /tmp

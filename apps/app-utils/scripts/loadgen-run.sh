#!/bin/bash

export RUSTFLAGS="-Awarnings"

cargo run --release --bin $1_client_bench -- \
   --gen-config ./scripts/gen_config.json \
   --output-path $2

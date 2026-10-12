#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/validate_rpcstack_boundary.py
cargo test -p rpcstack -p rpcstack-tonic -p rpcstack-sched -p tailclipper -p rajomon
cargo test -p paper-rpc --features orf
cargo run -p paper-rpc --features orf

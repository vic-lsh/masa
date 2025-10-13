#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 <rps> <duration>" >&2
  echo "Example: $0 500 2m" >&2
  exit 1
fi

if ! command -v timeout >/dev/null 2>&1; then
  echo "Error: timeout command not found" >&2
  exit 1
fi

RPS="$1"
DURATION="$2"

SIM_DIR="/home/jiexiao/research/masa-internal/apps/mssim/simulator"
TRACE_DIR="/home/jiexiao/research/masa-internal/trace-analysis/golden/S_86516878/"
CONFIG_DIR="/home/jiexiao/research/masa-internal/apps/mssim/simulator/service_configs"
OUTPUT_DIR="/home/jiexiao/research/masa-internal/apps/mssim/data/fifo"

mkdir -p "$OUTPUT_DIR"

echo "Running loadgen at ${RPS} RPS for ${DURATION}..."
(
  cd "$SIM_DIR"
  ROOT_LATENCY_OUTPUT_DIR="$OUTPUT_DIR" \
  RPS="$RPS" \
  timeout --signal=SIGINT --kill-after=30s --foreground "$DURATION" \
    cargo run -- --alibaba-trace "$TRACE_DIR" --config-dir "$CONFIG_DIR"
)


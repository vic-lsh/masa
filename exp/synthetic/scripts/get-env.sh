#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
APP_DIR=${MASA_APP_DIR:-"$(cd "$SCRIPT_DIR/../../.." && pwd)/apps/synthetic"}
GEN_CONFIG="$SCRIPT_DIR/gen_config.json"
LOCAL_CONFIG="$APP_DIR/scripts/local/config.docker.json"

if [[ ! -f "$GEN_CONFIG" ]]; then
    echo "Error: expected load generator config at '$GEN_CONFIG'" >&2
    exit 1
fi

if [[ ! -f "$LOCAL_CONFIG" ]]; then
    echo "Error: expected local config at '$LOCAL_CONFIG'" >&2
    exit 1
fi

FRONTEND_PORT=$(jq -r ".Addr" "$GEN_CONFIG" | sed -r 's/.*:([0-9]+)$/\1/')
echo "FRONTEND_PORT=$FRONTEND_PORT"

CONSTANT_REPLICAS=$(jq -r ".child_constant_replicas // 1" "$LOCAL_CONFIG")
PRESAMPLED_REPLICAS=$(jq -r "[.child_presampled_services[][0]] | add // 0" "$LOCAL_CONFIG")
RANDOM_REPLICAS="1"
CHILD_REPLICAS=$((CONSTANT_REPLICAS + RANDOM_REPLICAS + PRESAMPLED_REPLICAS))
echo "CONSTANT_REPLICAS=$CONSTANT_REPLICAS"
echo "PRESAMPLED_REPLICAS=$PRESAMPLED_REPLICAS"
echo "RANDOM_REPLICAS=$RANDOM_REPLICAS"
echo "CHILD_REPLICAS=$CHILD_REPLICAS"
CPUS_PER_REPLICA=$(jq -r ".child_cpus_per_replica // 1" "$LOCAL_CONFIG")
echo "CPUS_PER_REPLICA=$CPUS_PER_REPLICA"

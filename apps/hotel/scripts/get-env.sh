#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GEN_CONFIG="$SCRIPT_DIR/gen_config.json"

if [[ ! -f "$GEN_CONFIG" ]]; then
    echo "Error: expected load generator config at '$GEN_CONFIG'" >&2
    exit 1
fi

FRONTEND_PORT=$(jq -r '.Addr' "$GEN_CONFIG" | sed -r 's/.*:([0-9]+)$/\1/')
echo "FRONTEND_PORT=$FRONTEND_PORT"

HOTEL_CONFIG_PATH=${HOTEL_CONFIG:-"$SCRIPT_DIR/local/hotel.json"}
DEFAULT_REPLICAS=1

if [[ ! -f "$HOTEL_CONFIG_PATH" ]]; then
    echo "Error: expected hotel config at '$HOTEL_CONFIG_PATH'" >&2
    exit 1
fi

services=(rate profile reservation geo search user recommendation review)

for service_key in "${services[@]}"; do
    env_key="$(echo "${service_key}_replicas" | tr '[:lower:]' '[:upper:]')"
    replicas=$(jq -r --argjson default_replicas "$DEFAULT_REPLICAS" ".${service_key}.replicas // \$default_replicas" "$HOTEL_CONFIG_PATH")
    echo "${env_key}=${replicas}"
done

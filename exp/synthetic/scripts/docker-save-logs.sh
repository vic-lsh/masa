#!/bin/bash

set -e

# Script to write docker container logs for each service to a file

output_path=""
follow="false"

while [[ $# -gt 0 ]]; do
    case $1 in
    --follow)
        follow="true"
        shift 1
        ;;
    --output)
        output_path=$2
        shift 2
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

if [[ -z "$output_path" ]]; then
    echo "Error: --output must be provided" >&2
    exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
APP_DIR=${MASA_APP_DIR:-"$(cd "$SCRIPT_DIR/../../.." && pwd)/apps/synthetic"}
ENV_FILE="$APP_DIR/scripts/local/.env"

if [[ ! -f "$ENV_FILE" ]]; then
    echo "Error: expected environment file at '$ENV_FILE'" >&2
    exit 1
fi

# shellcheck disable=SC1090
source "$ENV_FILE"

declare -a container_names=(
  "synthetic_frontend"
)

for i in $(seq 1 "$CHILD_REPLICAS"); do
  container_names+=("local-child-service-$i")
done

mkdir -p "$output_path"
for name in "${container_names[@]}"; do
  if [[ "$follow" = "true" ]]; then
      docker logs -f "$name" &> "$output_path/$name.log" &
  else
      docker logs "$name" &> "$output_path/$name.log"
  fi
done

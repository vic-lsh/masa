#!/bin/bash

set -e

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
COMMON_DIR="$SCRIPT_DIR/../../common/scripts"
APP_DIR=${MASA_APP_DIR:-"$(cd "$SCRIPT_DIR/../../.." && pwd)/apps/synthetic"}

"$COMMON_DIR/docker-run.sh" --app synthetic "$@"

# configure CPU shares of presampled replicas
ENV_FILE="$APP_DIR/scripts/local/.env"
CONFIG_FILE="$APP_DIR/scripts/local/config.docker.json"

if [[ -f "$ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$ENV_FILE"
else
    echo "Warning: expected env file at '$ENV_FILE'; skipping CPU share updates" >&2
    exit 0
fi

if [[ ! -f "$CONFIG_FILE" ]]; then
    echo "Warning: expected config at '$CONFIG_FILE'; skipping CPU share updates" >&2
    exit 0
fi

i=$((CONSTANT_REPLICAS + RANDOM_REPLICAS + 1))

jq -r '.child_presampled_services[] | "\(.[0]) \(.[1] // 1)"' "$CONFIG_FILE" | while read -r replicas share; do
    for _ in $(seq 1 "$replicas"); do
        docker update --cpus="$share" "local-child-service-$i"
        i=$((i + 1))
    done
done

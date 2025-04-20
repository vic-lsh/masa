#!/bin/bash

set -e

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
    echo "Error: please run in the apps/hotel directory" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

skip_build=false
features=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --features)
        features="$2"
        shift 2
        ;;
    --skip-build)
        skip_build=true
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

if [[ "$skip_build" == false ]]; then
    if [[ -z "$features" ]]; then
        ./scripts/docker-build.sh
    else
        ./scripts/docker-build.sh --features $features
    fi
fi

echo "Service build complete. Starting services..."

docker compose -f $SCRIPT_DIR/local/containers+svcs.yaml up -d

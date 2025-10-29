#!/bin/bash

set -e

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
COMMON_DIR="$SCRIPT_DIR/../../common/scripts"

skip_build=false
build_args=()
forward_args=()

while [[ $# -gt 0 ]]; do
    case $1 in
    --skip-build)
        skip_build=true
        shift 1
        ;;
    --features|--rust-log)
        if [[ $# -lt 2 ]]; then
            echo "Error: $1 requires an argument" >&2
            exit 1
        fi
        build_args+=("$1" "$2")
        forward_args+=("$1" "$2")
        shift 2
        ;;
    --app|--app-dir)
        # Wrapper manages the application selection; drop any overrides
        if [[ $# -lt 2 ]]; then
            echo "Error: $1 requires an argument" >&2
            exit 1
        fi
        shift 2
        ;;
    *)
        forward_args+=("$1")
        shift 1
        ;;
    esac
done

if [[ "$skip_build" == false ]]; then
    "$SCRIPT_DIR/docker-build.sh" "${build_args[@]}"
fi

exec "$COMMON_DIR/docker-run.sh" --app hotel --skip-build "${forward_args[@]}"

#!/bin/bash

prefix=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --prefix)
        prefix="$2"
        shift 2
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

if [[ ! -z "$prefix" ]]; then
    export CONTAINER_PREFIX=$prefix
fi

docker compose -f ./scripts/local/containers+svcs.yaml down

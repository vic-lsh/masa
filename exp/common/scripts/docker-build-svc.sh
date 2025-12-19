#!/bin/bash
set -e

binary=""
features=""
rust_log="warn"
app=""
app_config_file="config.docker.json"
no_cache=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --binary)
        binary="$2"
        shift 2
        ;;
    --features)
        features="$2"
        shift 2
        ;;
    --rust-log)
        rust_log="$2"
        shift 2
        ;;
    --app)
        app="$2"
        shift 2
        ;;
    --app-config)
        app_config_file="$2"
        shift 2
        ;;
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

pwd=$(pwd)
if [[ "$pwd" != */apps/${app} ]]; then
    echo "Error: please run in the apps/$app directory" >&2
    exit 1
fi

# Validate required arguments
if [[ -z "$binary" ]]; then
    echo "Error: --binary argument is required"
    exit 1
fi

# go to project root. Dockerfile needs context from project root.
cd ../..

echo "Building docker image for service $binary. features: '$features'."
docker build -f ./apps/scripts/Dockerfile \
    --build-arg FEATURES=$features \
    --build-arg BINARY_NAME=$binary \
    --build-arg LOG_LEVEL=$rust_log \
    --build-arg APP=$app \
    --build-arg APP_CONFIG_FILE=$app_config_file \
    --ulimit nofile=4096:4096 \
    $no_cache \
    -t $binary \
    .

#!/bin/bash

set -e

skip_build=false
features=""
rust_log="info"
app=""
app_dir=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --features)
        features="$2"
        shift 2
        ;;
    --rust-log)
        rust_log="$2"
        shift 2
        ;;
    --skip-build)
        skip_build=true
        shift 1
        ;;
    --app)
        app="$2"
        shift 2
        ;;
    --app-dir)
        app_dir="$2"
        shift 2
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

if [[ -z "$app_dir" ]]; then
    if [[ -n "$app" ]]; then
        repo_root=$(git rev-parse --show-toplevel)
        app_dir="$repo_root/apps/$app"
    else
        pwd=$(pwd)
        if [[ "$pwd" == */apps/* ]]; then
            app=$(basename "$pwd")
            app_dir="$pwd"
        else
            echo "Error: unable to determine application directory. Pass --app or run inside apps/<app>." >&2
            exit 1
        fi
    fi
fi

if [[ ! -d "$app_dir" ]]; then
    echo "Error: application directory '$app_dir' not found" >&2
    exit 1
fi

pushd "$app_dir" >/dev/null
trap 'popd >/dev/null' EXIT

if [[ "$skip_build" == false ]]; then
    if [[ -z "$features" ]]; then
        ./scripts/docker-build.sh --rust-log "$rust_log"
    else
        ./scripts/docker-build.sh --rust-log "$rust_log" --features "$features"
    fi
fi

echo "Service build complete. Starting services..."

docker compose -f ./scripts/local/containers+svcs.yaml down
docker volume prune -a -f

docker compose -f ./scripts/local/containers+svcs.yaml up -d

trap - EXIT
popd >/dev/null

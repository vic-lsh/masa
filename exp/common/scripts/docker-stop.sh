#!/bin/bash

set -e

app="${MASA_APP_NAME:-}"
app_dir=""

while [[ $# -gt 0 ]]; do
    case $1 in
    --app)
        app=$2
        shift 2
        ;;
    --app-dir)
        app_dir=$2
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
docker compose -f ./scripts/local/containers+svcs.yaml down
trap - EXIT
popd >/dev/null

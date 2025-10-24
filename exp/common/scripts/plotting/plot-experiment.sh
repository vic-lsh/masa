#!/bin/bash

set -e

if [[ -z "${1:-}" ]]; then
    echo "Usage: ./scripts/plot-experiment.sh <experiment-name>" >&2
    exit 1
fi

experiment=$1
repo_root=$(git rev-parse --show-toplevel)
pwd=$(pwd)

case "$pwd" in
    *"/exp/"*)
        app=$(basename "$pwd")
        ;;
    *"/apps/"*)
        app=$(basename "$pwd")
        ;;
    *)
        echo "Error: please run this script from within exp/<app> or apps/<app>" >&2
        exit 1
        ;;
esac

data_root="$repo_root/exp/$app/data"
config_dir="$data_root/in/$experiment"
output_dir="$data_root/plots/$experiment"
trace_dir="$data_root/out/$experiment"

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

mkdir -p "$output_dir"

python3 "$SCRIPT_DIR/all.py" --config-dir "$config_dir" --data-dir "$trace_dir" --output-dir "$output_dir"

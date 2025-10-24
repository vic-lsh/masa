#!/bin/bash

set -e

if [[ -z "${1:-}" ]]; then
    echo "Usage: ./scripts/run-experiment.sh <experiment-name> [--plot]" >&2
    exit 1
fi

repo_root=$(git rev-parse --show-toplevel)
pwd=$(pwd)

case "$pwd" in
    *"/exp/"*)
        app=$(basename "$pwd")
        exp_dir="$pwd"
        ;;
    *"/apps/"*)
        app=$(basename "$pwd")
        exp_dir="$repo_root/exp/$app"
        ;;
    *)
        echo "Error: please run this script from within either exp/<app> or apps/<app>" >&2
        exit 1
        ;;
esac

if [[ ! -d "$exp_dir" ]]; then
    echo "Error: expected experiment directory at '$exp_dir'" >&2
    exit 1
fi

app_dir="$repo_root/apps/$app"
if [[ ! -d "$app_dir" ]]; then
    echo "Error: expected application directory at '$app_dir'" >&2
    exit 1
fi

exp_scripts_dir="$exp_dir/scripts"
app_scripts_dir="$app_dir/scripts"
experiment=$1
shift 1
plot="false"
app_config_filename="config.docker.json"
app_config_required="false"

if [[ "$app" == "hotel" ]]; then
    app_config_filename="hotel.json"
    app_config_required="true"
fi

app_config_dest="$app_scripts_dir/local/$app_config_filename"
app_env_path="$app_scripts_dir/local/.env"
exp_data_dir="$exp_dir/data"

mkdir -p "$exp_scripts_dir"
mkdir -p "$app_scripts_dir/local"

while [[ $# -gt 0 ]]; do
    case $1 in
    --plot)
        plot="true"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

in_dir="$exp_data_dir/in/$experiment"
out_dir="$exp_data_dir/out/$experiment"
plot_dir="$exp_data_dir/plots/$experiment"

if [[ ! -d "$in_dir" ]]; then
    echo "expected configuration for experiment at '$in_dir'"
    exit 1
fi

mkdir -p "$out_dir"
curr_ts=$(date +%s)
backup=/tmp/masa-save-$curr_ts
mkdir -p "$backup"
# save previous config
if [[ -f "$exp_scripts_dir/gen_config.json" ]]; then
    cp "$exp_scripts_dir/gen_config.json" "$backup"
fi
if [[ -f "$app_config_dest" ]]; then
    cp "$app_config_dest" "$backup"
fi
# load gen config and app config (if present) from $experiment
cp "$in_dir/gen_config.json" "$exp_scripts_dir/"
if [[ "$app_config_required" == "true" ]]; then
    if [[ ! -f "$in_dir/$app_config_filename" ]]; then
        echo "expected $app configuration at '$in_dir/$app_config_filename'"
        exit 1
    fi
    cp "$in_dir/$app_config_filename" "$app_config_dest"
elif [[ -f "$in_dir/$app_config_filename" ]]; then
    cp "$in_dir/$app_config_filename" "$app_config_dest"
fi
# save old output just in case
cp -r "$out_dir" "$backup" 2>/dev/null || true
# clear $out_dir and $plot_dir
rm -rf "$out_dir"/*
rm -rf "$plot_dir"
mkdir -p "$plot_dir"

policies=$(cat "$in_dir/policies" | tr -d '\n')
repeat=$(jq -r ".Repeats" "$exp_scripts_dir/gen_config.json")

export MASA_APP_DIR="$app_dir"
export MASA_APP_NAME="$app"

for i in $(seq 0 $((repeat - 1))); do
    echo "---------- iteration $i ----------"
    for policy in $policies; do
        echo "***** policy = $policy *****"
        mkdir -p "$(dirname "$app_env_path")"
        : > "$app_env_path"
        if [[ -f "$exp_scripts_dir/get-env.sh" ]]; then
            "$exp_scripts_dir/get-env.sh" > "$app_env_path"
        fi
        "$exp_scripts_dir/docker-run.sh" --features "$policy"

        "$exp_scripts_dir/loadgen-run.sh" --output "$out_dir/$i/$policy" --save-logs &
        loadgen_pid=$!

        # set up log file pipes so that container logs stream in as they run
        "$exp_scripts_dir/docker-save-logs.sh" --output "$out_dir/$i/$policy" --follow

        wait $loadgen_pid

        "$exp_scripts_dir/docker-stop.sh"
        rm -f "$app_env_path"
    done
done
touch "$out_dir/done"

if [[ "$plot" = "true" ]]; then
    "$repo_root/exp/common/scripts/plotting/plot-experiment.sh" "$experiment"
fi

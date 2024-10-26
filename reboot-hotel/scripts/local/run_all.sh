#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

features=""
while [[ "$#" -gt 0 ]]; do
    case $1 in
    --features)
        features="$2"
        shift
        ;;
    *)
        echo "Unknown parameter passed: $1"
        exit 1
        ;;
    esac
    shift
done

if [ -z "$features" ]; then
    echo "Error: must specify a masa feature flag using --features."
    exit 1
fi

# Restart the containers.
cd $current_dir/scripts/local
docker compose -f containers.yaml down
cd $current_dir/scripts/local
docker compose -f containers.yaml up -d

cd $current_dir



output="snippets/$features"

session_name="hotel"
tmux new-session -d -s $session_name -n "local"
tmux set-option -s pane-border-status top
tmux set-option -s pane-border-format "#{pane_title}"

services=(
    "hotel_geo"
    "hotel_rate"
    "hotel_search"
    "hotel_profile"
    "hotel_frontend"
    "hotel_client_bench"
)
waits_secs=(
    0
    0
    6
    0
    12
    18
)
rust_log=warn

first_pane=true

for i in "${!services[@]}"; do
    service=${services[$i]}
    wait_secs=${waits_secs[$i]}

    if [[ "$service" != "hotel_client_bench" ]]; then
        run_cmd=" \
        RUST_LOG=$rust_log \
        cargo run --release \
        --features $features \
        --bin $service \
        -- \
        --config scripts/local/hotel_config.json \
        > $output/tmp_$service.log 2>&1"
    else
        run_cmd=" \
        RUST_LOG=$rust_log \
        cargo run --release \
        --features $features \
        --bin $service \
        -- \
        --hotel-config scripts/local/hotel_config.json \
        --gen-config $output/gen_config.json \
        > $output/tmp_${service}.log 2>&1"
    fi

    cmd=" \
    cd ${current_dir}; \
    sleep $wait_secs; \
    $run_cmd \
    "

    if [ "$first_pane" = true ]; then
        tmux select-pane -T $service
        first_pane=false
    else
        tmux split-window -h -t $session_name
        tmux select-pane -T $service
        tmux select-layout -t $session_name tiled
    fi
    tmux send-keys -t $session_name "$cmd" C-m
done

tmux attach -t $session_name

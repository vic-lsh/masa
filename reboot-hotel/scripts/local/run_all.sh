#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

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
rust_log=warn
rps=100
secs=10
concurrency=128

first_pane=true

for i in "${!services[@]}"; do
    service=${services[$i]}
    wait_secs=$((i * 5))

    if [[ "$service" != "hotel_client_bench" ]]; then
        run_cmd=" \
        RUST_LOG=$rust_log \
        cargo run --release \
        --features prio_global \
        --bin $service \
        -- \
        --config scripts/local/config.json \
        > tmp_$service.log 2>&1 \
        "
    else
        run_cmd=" \
        RUST_LOG=$rust_log \
        cargo run --release \
        --bin $service \
        -- \
        --rps $rps \
        --secs $secs \
        --concurrency $concurrency \
        --output tmp_$service.csv \
        > tmp_$service.log 2>&1 \
        "
    fi

    cmd=" \
    cd ~/Masa-Lo-Ding/reboot-hotel; \
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

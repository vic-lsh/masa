#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

features=""
output=""
while [[ "$#" -gt 0 ]]; do
    case $1 in
    --features)
        features="$2"
        shift
        ;;
    --output)
        output="$2"
        shift
        ;;
    *)
        echo "Unknown parameter passed: $1"
        exit 1
        ;;
    esac
    shift
done

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
repeats=2
rps=200
secs=60
concurrency=128

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
        --config scripts/local/config.json \
        > $output/tmp_$service.log 2>&1
        "
    else
        run_cmd=""
        for i in $(seq 0 $(($repeats - 1))); do
            run_cmd+=" \
            RUST_LOG=$rust_log \
            cargo run --release \
            --bin $service \
            -- \
            --config scripts/local/config.json \
            --rps $rps \
            --secs $secs \
            --concurrency $concurrency \
            --output $output/${service}_$i.csv \
            > $output/tmp_${service}_$i.log 2>&1; "
        done
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

#!/bin/bash

current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

features=""
output=""
repeats="1"
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
    --repeats)
        repeats="$2"
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
if [ -z "$output" ]; then
    output=snippets/$features
fi

cd $current_dir

session_name="hotel"

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
    10
    0
    20
    26
)
rust_log=warn

ready_go() {
    run_idx=$1
    rm $output/*.log

    docker compose -f scripts/local/containers.yaml down
    docker compose -f scripts/local/containers.yaml up -d

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
--run-idx $run_idx \
> $output/tmp_$service.log 2>&1"

        fi

        cmd=" \
cd $current_dir; \
sleep $wait_secs; \
$run_cmd"

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

    all_done=false
    while [[ $all_done == false ]]; do
        sleep 10
        service=${services[-1]}
        if [ ! -f $output/tmp_$service.log ]; then
            continue
        fi
        if tail -n 1 $output/tmp_$service.log | grep -q "Load generator done"; then
            all_done=true
        fi
    done
}

for ((run = 0; run < repeats; run++)); do
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    echo "Starting run $run/$repeats..."

    tmux new-session -d -s $session_name -n "local"
    tmux set-option -s pane-border-status top
    tmux set-option -s pane-border-format "#{pane_title}"

    ready_go $run
done

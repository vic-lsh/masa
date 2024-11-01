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

reset() {
    rm $output/*.log
    docker compose -f scripts/local/containers.yaml down -v --remove-orphans
    docker compose -f scripts/local/containers.yaml up -d
}

ready_go() {
    run_idx=$1

    first_pane=true

    for i in "${!services[@]}"; do
        service=${services[$i]}
        wait_secs=${waits_secs[$i]}

        if [[ "$service" == "hotel_client_bench" ]]; then
            run_cmd=" \
RUST_LOG=$rust_log \
cargo run --release \
--features $features \
--bin $service \
-- \
--hotel-config scripts/local/hotel_config.json \
--gen-config $output/gen_config.json \
--run-idx $run_idx;\
sleep 3; \
tmux kill-session"

            #       elif [[ "$service" == "hotel_rate" ]]; then
            #
            #            run_cmd=" \
            # cargo build --release --features $features --bin $service; \
            # RUST_LOG=$rust_log \
            # timeout 120s perf record -g --call-graph dwarf ../target/release/$service \
            # --config scripts/local/hotel_config.json; \
            # perf script | inferno-collapse-perf > stacks.$service.$features.folded"

            #             run_cmd=" \
            # RUST_LOG=$rust_log \
            # cargo flamegraph \
            # --features $features \
            # --bin $service \
            # -- \
            # --config scripts/local/hotel_config.json"

            #             run_cmd=" \
            # cargo build --release \
            # --features $features \
            # --bin $service && \
            # sudo RUST_LOG=$rust_log \
            # perf record --call-graph dwarf ../target/release/$service \
            # --config scripts/local/hotel_config.json"

        else

            run_cmd=" \
RUST_LOG=$rust_log \
cargo run --release \
--features $features \
--bin $service \
-- \
--config scripts/local/hotel_config.json"

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
}

for ((run = 0; run < repeats; run++)); do
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    echo "Starting run $run/$repeats..."
    reset

    tmux new-session -d -s $session_name -n "local"
    tmux set-option -s pane-border-status top
    tmux set-option -s pane-border-format "#{pane_title}"

    ready_go $run
done

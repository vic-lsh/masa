#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

rust_log=warn
tracker_capacity=512
pctl_deadline=""
pctl_latest_exec=""
cargo_features=""
output_path=""
gen_config=""
hotel_config=""
repeats=1
while [[ "$#" -gt 0 ]]; do
    case $1 in
    --rust-log)
        rust_log="$2"
        shift
        ;;
    --tracker-capacity)
        tracker_capacity="$2"
        shift
        ;;
    --pctl-deadline)
        pctl_deadline="$2"
        shift
        ;;
    --pctl-latest-exec)
        pctl_latest_exec="$2"
        shift
        ;;
    --cargo-features)
        cargo_features="$2"
        shift
        ;;
    --output-path)
        output_path="$2"
        shift
        ;;
    --gen-config)
        gen_config="$2"
        shift
        ;;
    --hotel-config)
        hotel_config="$2"
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
if [ -z "$pctl_deadline" ]; then
    echo "Expected a percentile of deadline using --pctl-deadline"
    exit 1
fi
if [ -z "$pctl_latest_exec" ]; then
    echo "Expected a percentile of execution using --pctl-latest-exec"
    exit 1
fi
if [ -z "$cargo_features" ]; then
    echo "Expected a masa feature flag using --cargo_features"
    exit 1
fi
if [ -z "$output_path" ]; then
    echo "Expected an output path using --output-path"
    exit 1
fi
if [ -z "$gen_config" ]; then
    echo "Expected a gen config file using --gen-config"
    exit 1
fi
if [ -z "$hotel_config" ]; then
    echo "Expected a hotel config file using --hotel-config"
    exit 1
fi

session_name=hotel
services=(
    # hotel_geo
    # hotel_rate
    # hotel_search
    # hotel_profile
    # hotel_reservation
    # hotel_user
    # hotel_frontend
    hotel_client_bench
)
waits_secs=(
    # 0
    # 0
    # 15
    # 0
    # 0
    # 0
    # 18
    # 21
    0
)

init() {
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    mkdir -p $output_path
}

build() {
    echo "Building $cargo_features..."
    cargo build \
        --release \
        --features $cargo_features \
        >$output_path/tmp_build.log 2>&1
}

reset() {
    rm $output_path/*.log
    docker compose -f scripts/local/containers.yaml down --remove-orphans
    # docker compose -f scripts/local/containers.yaml up -d
}

cleanup() {
    echo "Cleaning up..."
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    exit 1
}

ready_go() {
    run_idx=$1

    first_pane=true

    for i in "${!services[@]}"; do
        service=${services[$i]}
        wait_secs=${waits_secs[$i]}

        if [[ "$service" != "hotel_client_bench" ]]; then

            run_cmd=" \
RUST_LOG=$rust_log \
TRACKER_CAPACITY=$tracker_capacity \
PCTL_DEADLINE=$pctl_deadline \
PCTL_LATEST_EXEC=$pctl_latest_exec \
cargo run --release \
--features $cargo_features \
--bin $service \
-- \
--config $hotel_config \
> $output_path/tmp_$service.log 2>&1"

        else

            run_cmd=" \
RUST_LOG=$rust_log \
TRACKER_CAPACITY=$tracker_capacity \
PCTL_DEADLINE=$pctl_deadline \
PCTL_LATEST_EXEC=$pctl_latest_exec \
cargo run --release \
--features $cargo_features \
--bin $service \
-- \
--gen-config $gen_config \
--hotel-config $hotel_config \
--output-path $output_path \
--run-idx $run_idx \
> $output_path/tmp_$service.log 2>&1"

        fi

        cmd=" \
cd $pwd; \
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

    all_done=false
    while [[ $all_done == false ]]; do
        sleep 10
        service=${services[-1]}
        if [ ! -f $output_path/tmp_$service.log ]; then
            continue
        fi
        if tail -n 1 $output_path/tmp_$service.log | grep -q "Load generator done"; then
            all_done=true
        fi
    done

    tmux kill-session -t $session_name
}

trap cleanup SIGINT SIGTERM
init
build

for ((run = 0; run < repeats; run++)); do
    echo "Starting run $run/$repeats..."
    reset

    tmux new-session -d -s $session_name -n "local"
    tmux set-option -s pane-border-status top
    tmux set-option -s pane-border-format "#{pane_title}"

    ready_go $run
done

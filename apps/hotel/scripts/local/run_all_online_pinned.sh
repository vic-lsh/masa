#!/bin/bash
current_dir=$(pwd)
if [[ "$current_dir" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

# CPU configuration
START_CPU=${START_CPU:-0}        # Default starting CPU is 0
CPUS_PER_SERVICE=${CPUS_PER_SERVICE:-4}  # Default 4 CPUs per service

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
    echo "Expected a masa feature flag using --features"
    exit 1
fi

if [ -z "$output" ]; then
    output=snippets/$features
fi

session_name="hotel"
services=(
    "hotel_geo"
    "hotel_rate"
    "hotel_search"
    "hotel_profile"
    "hotel_frontend"
    "hotel_client_bench"
)

# Calculate total CPUs needed
TOTAL_CPUS=$((${#services[@]} * CPUS_PER_SERVICE))
MAX_CPU=$((START_CPU + TOTAL_CPUS - 1))

# Verify system has enough CPUs
AVAILABLE_CPUS=$(nproc)
if [ $MAX_CPU -ge $AVAILABLE_CPUS ]; then
    echo "Error: Not enough CPU cores available" >&2
    echo "Required CPUs: $START_CPU-$MAX_CPU (total: $TOTAL_CPUS)" >&2
    echo "Available CPUs: 0-$((AVAILABLE_CPUS-1)) (total: $AVAILABLE_CPUS)" >&2
    exit 1
fi

# Generate CPU assignments dynamically
declare -a cpu_assignments
for i in "${!services[@]}"; do
    start=$((START_CPU + i * CPUS_PER_SERVICE))
    end=$((start + CPUS_PER_SERVICE - 1))
    cpu_assignments[$i]="$start-$end"
done

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
        cpu_range=${cpu_assignments[$i]}
        
        if [[ "$service" == "hotel_client_bench" ]]; then
            run_cmd=" \
RUST_LOG=$rust_log \
taskset -c $cpu_range \
cargo run --release \
--features $features \
--bin $service \
-- \
--hotel-config scripts/local/hotel_config.json \
--gen-config $output/gen_config.json \
--run-idx $run_idx;\
sleep 3; \
tmux kill-session"
        else
            run_cmd=" \
RUST_LOG=$rust_log \
taskset -c $cpu_range \
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
            tmux select-pane -T "$service (CPUs $cpu_range)"
            first_pane=false
        else
            tmux split-window -h -t $session_name
            tmux select-pane -T "$service (CPUs $cpu_range)"
            tmux select-layout -t $session_name tiled
        fi
        tmux send-keys -t $session_name "$cmd" C-m
    done
    tmux attach -t $session_name
}

cargo build --release --features $features

# Print CPU assignment information
echo "CPU Configuration:"
echo "Starting CPU: $START_CPU"
echo "CPUs per service: $CPUS_PER_SERVICE"
echo "Total CPUs needed: $TOTAL_CPUS"
echo "CPU assignments:"
for i in "${!services[@]}"; do
    echo "  ${services[$i]}: CPUs ${cpu_assignments[$i]}"
done
echo

for ((run = 0; run < repeats; run++)); do
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    echo "Starting run $run/$repeats..."
    # reset
    tmux new-session -d -s $session_name -n "local"
    tmux set-option -s pane-border-status top
    tmux set-option -s pane-border-format "#{pane_title}"
    ready_go $run
done

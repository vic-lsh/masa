#!/bin/bash

services=(
    "hotel_frontend"
    "hotel_geo"
    "hotel_rate"
    "hotel_search"
    "hotel_profile"
    "hotel_reservation"
    "hotel_user"
)

pwd=$(pwd)
if [[ "$pwd" != */apps/hotel ]]; then
    echo "Error: plese run in the apps/hotel directory" >&2
    exit 1
fi

features=""
parallel=0

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --features)
        features="$2"
        shift 2
        ;;
    --par)
        parallel=1
        shift 1
        ;;
    *)
        echo "Unknown argument: $1"
        exit 1
        ;;
    esac
done

# Validate required arguments
if [[ -z "$features" ]]; then
    echo "Warn: --features not set."
fi

echo "Building all hotel services. Feature flags: $features."

if [[ -z "$features" ]]; then
    cmd="cargo build --release"
else
    cmd="cargo build --release --features $features"
fi
output=$(eval "$cmd" 2>&1)
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Build failed with exit code $exit_code"
    echo "Output:"
    echo "$output"
    exit $exit_code
fi

if [ $parallel -eq 0 ]; then
    echo "Building docker images sequentially."

    set -e
    for svc in "${services[@]}"; do
        ./scripts/docker-build-svc.sh --binary $svc
    done

else
    echo "Building docker images in parallel."

    declare -A svc_pid_map

    declare -a pids

    # Function to handle errors
    handle_error() {
        local pid=$1
        local exit_status=$2
        # Look up the original svcing that caused the error
        for svc in "${!svc_pid_map[@]}"; do
            if [ "${svc_pid_map[$svc]}" == "$pid" ]; then
                echo "Failed to build docker image for $svc" >&2
                break
            fi
        done
    }

    build_docker_img() {
        local svc=$1
        local pid=$$
        if ./scripts/docker-build-svc.sh --binary "$svc"; then
            #echo "Process $pid completed successfully"
            return 0
        else
            echo "Process $pid failed"
            return 1
        fi
    }

    # Build docker images in parallel
    for svc in "${services[@]}"; do
        build_docker_img "$svc" >/dev/null 2>&1 &
        pid=$!
        pids+=($pid)
        svc_pid_map[$svc]=$pid
        echo "Started building docker image for service '$svc' at pid $pid."
    done

    failed=0

    # Wait for all processes to complete and check their exit status
    for pid in "${pids[@]}"; do
        if ! wait $pid; then
            handle_error $pid $?
            failed=1
        fi
    done

    echo "-----------------------------------"
    if [ $failed -eq 1 ]; then
        echo "One or more processes failed" >&2
        exit 1
    else
        echo "All processes completed successfully"
        exit 0
    fi
fi

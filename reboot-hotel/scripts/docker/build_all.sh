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
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

tag=""
features=""
parallel=0

whoami=$(whoami)
if [[ "$whoami" == "wxdeng" ]]; then
    docker_username="dengwxn"
else
    echo "Error: unknown user name" >&2
    exit 1
fi

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --tag)
        tag="$2"
        shift 2
        ;;
    --features)
        features="$2"
        shift 2
        ;;
    --parallel)
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
if [[ -z "$tag" ]]; then
    echo "Error: --tag argument is required"
    exit 1
fi
if [[ -z "$features" ]]; then
    echo "Error: --features argument is required"
    exit 1
fi

echo "Compiling hotel services with features $features..."
if [[ -z "$features" ]]; then
    cmd="cargo build --release"
else
    cmd="cargo build --release --features $features"
fi
output=$(eval "$cmd" 2>&1)
exit_code=$?
if [ $exit_code -ne 0 ]; then
    echo "Failed to compile hotel services"
    echo "Output:"
    echo "$output"
    exit $exit_code
fi

if [ $parallel -eq 0 ]; then
    echo "Building docker images sequentially..."

    set -e
    for svc in "${services[@]}"; do
        ./scripts/docker/build.sh --tag $tag --binary $svc
    done

    for svc in "${services[@]}"; do
        docker tag $svc:$tag $docker_username/$svc:$tag
        docker push $docker_username/$svc:$tag
    done
else
    echo "Building docker images in parallel..."

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

    build_and_push_docker_img() {
        local svc=$1
        local pid=$$
        if ./scripts/docker/build.sh --tag $tag --binary "$svc"; then
            docker tag $svc:$tag $docker_username/$svc:$tag
            docker push $docker_username/$svc:$tag
            return 0
        else
            echo "Process $pid failed"
            return 1
        fi
    }

    # Build docker images in parallel
    for svc in "${services[@]}"; do
        build_and_push_docker_img "$svc" >/dev/null 2>&1 &
        pid=$!
        pids+=($pid)
        svc_pid_map[$svc]=$pid
        echo "Building docker image for service $svc at pid $pid..."
    done

    failed=0

    # Wait for all processes to complete and check their exit status
    for pid in "${pids[@]}"; do
        if ! wait $pid; then
            handle_error $pid $?
            failed=1
        fi
    done

    if [ $failed -eq 1 ]; then
        echo "Failed to build some of the services" >&2
        exit 1
    else
        echo "Successfully built all the services"
    fi
fi

echo "Successfully pushed all the docker images with tag $tag"

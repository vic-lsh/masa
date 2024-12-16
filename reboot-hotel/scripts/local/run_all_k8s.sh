#!/bin/bash

pwd=$(pwd)
if [[ "$pwd" != */reboot-hotel ]]; then
    echo "Error: plese run in the reboot-hotel directory" >&2
    exit 1
fi

# tracker_capacity=512
# pctl_deadline=50
# pctl_latest_exec=50

tag=""
folder=""
rust_log=warn
cargo_features=""
gen_config=""
hotel_config=""
output_path=""
session_name=hotel

while [[ "$#" -gt 0 ]]; do
    case $1 in
    --tag)
        tag="$2"
        shift
        ;;
    --folder)
        folder="$2"
        shift
        ;;
    --rust-log)
        rust_log="$2"
        shift
        ;;
    --cargo-features)
        cargo_features="$2"
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
    --output-path)
        output_path="$2"
        shift
        ;;
    *)
        echo "Unknown parameter passed: $1"
        exit 1
        ;;
    esac
    shift
done
if [ -z "$tag" ]; then
    echo "Expected a tag using --tag"
    exit 1
fi
if [ -z "$folder" ]; then
    echo "Expected a folder using --folder"
    exit 1
fi
if [ -z "$cargo_features" ]; then
    echo "Expected cargo features using --cargo-features"
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
if [ -z "$output_path" ]; then
    echo "Expected an output path using --output-path"
    exit 1
fi

clean_up_session() {
    echo "Cleaning up session..."
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    exit 1
}

trap clean_up_session SIGINT SIGTERM

init_all() {
    echo "Initializing all..."
    if tmux has-session -t $session_name 2>/dev/null; then
        tmux kill-session -t $session_name
    fi
    mkdir -p $output_path
    rm $output_path/*.log
    docker compose -f scripts/local/containers.yaml down --remove-orphans
    tmux new-session -d -s $session_name -n "local"
    tmux set-option -s pane-border-status top
    tmux set-option -s pane-border-format "#{pane_title}"
}

reset_k8s() {
    echo "Resetting k8s..."
    kubectl delete all --all
}

build_client() {
    echo "Building client with features $cargo_features..."
    cargo build \
        --release \
        --features $cargo_features \
        >$output_path/tmp_build.log 2>&1
}

build_services() {
    echo "Building services into docker images..."
    $pwd/scripts/docker/build_all.sh --tag $tag --features $cargo_features --parallel
}

preprocess_k8s_yaml() {
    echo "Preprocessing k8s yaml..."
    python3 $pwd/scripts/k8s-template/preprocess_yaml.py \
        --tag $tag \
        --config $folder/k8s_config.json \
        --input-path $pwd/scripts/k8s-template/yaml \
        --output-path $folder/$tag/yaml
}

deploy_k8s_yaml() {
    kill $(lsof -t -i:8660) >/dev/null 2>&1
    echo "Deploying k8s yaml..."
    kubectl apply -f $folder/$tag/yaml
    if ! kubectl rollout status deployment --timeout 360s; then
        kubectl get deployments
        echo "Failed to get deployments ready" >&2
        exit 1
    fi
    if ! kubectl wait --for=condition=Ready pods --all --timeout=360s; then
        kubectl get pods
        echo "Failed to get pods ready" >&2
        exit 1
    fi
    until kubectl get endpoints frontend-service -o jsonpath='{.subsets[*].addresses[*]}' | grep -q .; do
        echo "Waiting for endpoint frontend-service ready..."
        sleep 3
    done
    kubectl get endpoints
    echo "Waiting for cold war..."
    sleep 30
}

forward_k8s_port() {
    echo "Forwarding k8s port..."
    kubectl port-forward service/frontend-service 8660:8660
}

run_client() {
    echo "Running client..."

    first_pane=true

    service=hotel_client_bench
    waits_secs=0

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
--run-idx 0 \
> $output_path/tmp_$service.log 2>&1"

    cmd=" \
cd $pwd; \
sleep $wait_secs; \
$run_cmd"

    tmux select-pane -T $service
    first_pane=false
    tmux send-keys -t $session_name "$cmd" C-m

    done=false
    while [[ $done == false ]]; do
        sleep 6
        if [ ! -f $output_path/tmp_$service.log ]; then
            continue
        fi
        if tail -n 1 $output_path/tmp_$service.log | grep -q "Load generator done"; then
            done=true
        fi
    done

    tmux kill-session -t $session_name
}

reset_k8s &
init_all &
build_client &
# build_services &
preprocess_k8s_yaml &
wait

deploy_k8s_yaml
forward_k8s_port &
pid=$!
run_client
kill $pid
# reset_k8s

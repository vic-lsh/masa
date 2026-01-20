#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/synthetic"
exp_names=()
no_cache=""

# Parse arguments
deploy_mode_arg=""
while [[ $# -gt 0 ]]; do
    case $1 in
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    --deploy-mode)
        deploy_mode_arg="$2"
        shift 2
        ;;
    ci|ci_call_graph)
        exp_names+=("$1")
        shift 1
        ;;
    *)
        echo "Unknown argument: $1" >&2
        echo "Usage: $0 [ci|ci_call_graph] [--no-cache] [--deploy-mode <docker|k8s>]" >&2
        exit 1
        ;;
    esac
done

if [ ${#exp_names[@]} -eq 0 ]; then
    exp_names=("ci" "ci_call_graph")
fi

if [ ! -d "$exp_dir" ]; then
    echo "Synthetic experiment directory not found at $exp_dir" >&2
    exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the synthetic experiment test." >&2
    exit 1
fi

# Setup uv and virtual environment
if ! command -v uv >/dev/null 2>&1; then
    echo "Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
fi

echo "Syncing Python dependencies with uv..."
cd "$repo_root"
uv sync
source .venv/bin/activate

assert_path_exists() {
    if [ -e "$1" ]; then
        echo "File exists: $1"
    else
        echo "Expected path missing: $1" >&2
        exit 1
    fi
}

ensure_k8s_deps() {
    # Ensure local bin is in PATH
    mkdir -p "$HOME/.local/bin"
    export PATH="$HOME/.local/bin:$PATH"

    if ! command -v kubectl >/dev/null 2>&1; then
        echo "Installing kubectl..."
        curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
        chmod +x kubectl
        mv kubectl "$HOME/.local/bin/"
    fi

    if ! command -v kind >/dev/null 2>&1; then
        echo "Installing kind..."
        curl -Lo ./kind https://kind.sigs.k8s.io/dl/v0.24.0/kind-linux-amd64
        chmod +x ./kind
        mv ./kind "$HOME/.local/bin/"
    fi
}

setup_kind_cluster() {
    # Check if cluster exists
    if ! kind get clusters | grep -q "kind"; then
        echo "Creating kind cluster..."
        # Try to create, ignore failure (race condition with other parallel jobs)
        kind create cluster || true
    fi
    # We do NOT destroy the cluster on exit, as it may be shared by parallel jobs.
    # The CI environment or a dedicated cleanup job should handle cluster deletion.
}

# Generate a unique namespace for this test run
NAMESPACE="test-syn-$(date +%s)-$RANDOM"

cleanup_namespace() {
    if [ -n "$NAMESPACE" ]; then
        echo "Cleaning up namespace $NAMESPACE..."
        kubectl delete namespace "$NAMESPACE" --wait=false || true
    fi
}

run_test() {
    local exp_name="$1"
    local deploy_mode="$2"
    echo "--------------------------------------------------"
    echo "Running test for experiment: $exp_name (mode: $deploy_mode)"
    echo "--------------------------------------------------"

    local config_dir="$exp_dir/data/in/$exp_name"
    local gen_config="$config_dir/gen_config.json"
    local policies_file="$config_dir/policies"
    local out_dir="$exp_dir/data/out/$exp_name"

    if [ ! -d "$config_dir" ]; then
        echo "Experiment config not found at $config_dir" >&2
        exit 1
    fi

    if [ ! -f "$gen_config" ]; then
        echo "gen_config.json not found at $gen_config" >&2
        exit 1
    fi

    if [ ! -f "$policies_file" ]; then
        echo "policies not found at $policies_file" >&2
        exit 1
    fi

    echo "Cleaning previous experiment output at $out_dir"
    rm -rf "$out_dir"

    echo "Running synthetic experiment: $exp_name"
    cd "$repo_root"
    
    # Pass namespace if using k8s
    extra_args=""
    if [ "$deploy_mode" == "k8s" ]; then
        extra_args="--namespace $NAMESPACE"
        # Ensure namespace exists
        kubectl create namespace "$NAMESPACE" || true
        # Clean up namespace on exit
        trap cleanup_namespace EXIT
    fi
    
    ./exp/synthetic/scripts/run-experiment.sh "$exp_name" --deploy-mode "$deploy_mode" $no_cache $extra_args

    echo "Validating experiment output..."

    # Check that the done marker exists
    assert_path_exists "$out_dir/done"

    # Read policies from the policies file (whitespace-separated)
    read -ra policy_array <<< "$(tr '\n' ' ' < "$policies_file")"

    # Read RPS values, APIs, and repeats from gen_config.json
    mapfile -t rps_array < <(python -c 'import json,sys; print("\n".join(str(x) for x in json.load(open(sys.argv[1]))["Rps"]))' "$gen_config")
    mapfile -t api_array < <(python -c 'import json,sys; print("\n".join(str(x) for x in json.load(open(sys.argv[1]))["Apis"]))' "$gen_config")
    repeats="$(python -c 'import json,sys; print(int(json.load(open(sys.argv[1]))["Repeats"]))' "$gen_config")"

    # Validate output files for each repeat, policy, RPS, and API
    for i in $(seq 0 $((repeats - 1))); do
        for policy in "${policy_array[@]}"; do
            policy_out_dir="$out_dir/$i/$policy"
            assert_path_exists "$policy_out_dir"
            assert_path_exists "$policy_out_dir/loadgen.log"

            for rps in "${rps_array[@]}"; do
                for api in "${api_array[@]}"; do
                    expected_file="$policy_out_dir/r${rps}_${api}.csv"
                    echo "Checking: $expected_file"
                    assert_path_exists "$expected_file"
                done
            done
        done
    done

    echo "Generating plots..."
    cd "$repo_root"
    python -m exp.runner plot synthetic "$exp_name"

    echo "Test for $exp_name ($deploy_mode) passed."
}

deploy_modes=()
if [ -n "$deploy_mode_arg" ]; then
    deploy_modes+=("$deploy_mode_arg")
else
    deploy_modes+=("docker")
    # Only run k8s if kubectl is available
    if command -v kubectl >/dev/null 2>&1; then
        deploy_modes+=("k8s")
    fi
fi

# Ensure k8s tools and cluster are available if k8s mode is selected
for mode in "${deploy_modes[@]}"; do
    if [ "$mode" == "k8s" ]; then
        ensure_k8s_deps
        setup_kind_cluster
        break
    fi
done

for exp in "${exp_names[@]}"; do
    for mode in "${deploy_modes[@]}"; do
        run_test "$exp" "$mode"
    done
done

echo "All synthetic CI experiment tests passed."

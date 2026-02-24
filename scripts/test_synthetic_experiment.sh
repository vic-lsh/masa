#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/synthetic"
exp_names=()
no_cache=""
deploy_mode="docker"
deploy_args=""
source "$repo_root/scripts/kind_utils.sh"

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    --deploy-mode)
        deploy_mode="$2"
        shift 2
        ;;
    ci|ci_call_graph)
        exp_names+=("$1")
        shift 1
        ;;
    *)
        echo "Unknown argument: $1" >&2
        echo "Usage: $0 [ci|ci_call_graph] [--no-cache] [--deploy-mode <docker|kind>]" >&2
        exit 1
        ;;
    esac
done

if [ ${#exp_names[@]} -eq 0 ]; then
    exp_names=("ci")
fi

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the synthetic experiment test." >&2
    exit 1
fi

if [[ "$deploy_mode" == "kind" ]]; then
    deploy_args="--kind"
    # Use a unique cluster name to avoid conflicts in CI
    # If CI_JOB_ID is set (GitLab CI), use it; otherwise use a random suffix
    if [ -n "${CI_JOB_ID:-}" ]; then
        CLUSTER_NAME="kind-${CI_JOB_ID}"
    else
        CLUSTER_NAME="kind-$(date +%s)"
    fi
    echo "Using Kind cluster name: $CLUSTER_NAME"

    # Export KUBECONFIG to a dedicated path to avoid overwriting default config
    export KUBECONFIG="$HOME/.kube/config-$CLUSTER_NAME"
    # Export CLUSTER_NAME for the python runner to use
    export KIND_CLUSTER_NAME="$CLUSTER_NAME"

    if ! kind get clusters | grep -q "^$CLUSTER_NAME$"; then
        kind_node_image="$(kind_node_image_ulimit "$repo_root")"
        echo "Creating kind cluster: $CLUSTER_NAME..."
        kind create cluster --name "$CLUSTER_NAME" --image "$kind_node_image"
    else
        echo "Kind cluster $CLUSTER_NAME already exists."
    fi

    # Ensure we tear down the cluster on exit
    trap 'echo "Deleting kind cluster: $CLUSTER_NAME..."; kind delete cluster --name "$CLUSTER_NAME"; rm -f "$KUBECONFIG"' EXIT
elif [[ "$deploy_mode" == "docker" ]]; then
    deploy_args=""
else
    echo "Invalid deploy mode: $deploy_mode. Must be 'docker' or 'kind'." >&2
    exit 1
fi

if [ ! -d "$exp_dir" ]; then
    echo "Synthetic experiment directory not found at $exp_dir" >&2
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

run_test() {
    local exp_name="$1"
    echo "--------------------------------------------------"
    echo "Running test for experiment: $exp_name with mode: $deploy_mode"
    local out_dir="$exp_dir/out/$exp_name"
    echo "Cleaning previous experiment output at $out_dir"
    rm -rf "$out_dir"

    echo "Running synthetic experiment: $exp_name"
    cd "$repo_root"
    # We use unquoted variables for flags to allow empty strings to disappear
    # shellcheck disable=SC2086
    python -m exp_runner.runner run synthetic "$exp_name" $no_cache $deploy_args --smoke-test --plot

    echo "Test for $exp_name passed."
}

for exp in "${exp_names[@]}"; do
    run_test "$exp"
done

echo "All synthetic CI experiment tests passed."

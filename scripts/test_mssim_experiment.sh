#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
experiment_name="e2e_test"
out_dir="$repo_root/exp/mssim/data/out/$experiment_name"
deploy_mode="docker"
deploy_args=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --deploy-mode)
        deploy_mode="$2"
        shift 2
        ;;
    *)
        echo "Unknown argument: $1" >&2
        echo "Usage: $0 [--deploy-mode <docker|kind>]" >&2
        exit 1
        ;;
    esac
done

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the MSSIM experiment test." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to run the MSSIM experiment test." >&2
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
        echo "Creating kind cluster: $CLUSTER_NAME..."
        kind create cluster --name "$CLUSTER_NAME"
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

# Setup uv and virtual environment
if ! command -v uv >/dev/null 2>&1; then
    echo "Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
fi

# Only sync if not in a virtual environment (CI already does this)
if [ -z "${VIRTUAL_ENV:-}" ]; then
    echo "Syncing Python dependencies with uv..."
    cd "$repo_root"
    uv sync
    source .venv/bin/activate
else
    echo "Using existing virtual environment: $VIRTUAL_ENV"
    cd "$repo_root"
fi

echo "Cleaning previous experiment output at $out_dir"
rm -rf "$out_dir"

echo "Running MSSIM experiment using exp_runner.runner with mode: $deploy_mode"
# shellcheck disable=SC2086
python -m exp_runner.runner run mssim "$experiment_name" --rm-data --smoke-test $deploy_args

echo "MSSIM experiment smoke test passed."

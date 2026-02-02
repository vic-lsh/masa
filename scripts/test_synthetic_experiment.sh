#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/synthetic"
exp_names=()
no_cache=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    ci|ci_call_graph)
        exp_names+=("$1")
        shift 1
        ;;
    *)
        echo "Unknown argument: $1" >&2
        echo "Usage: $0 [ci|ci_call_graph] [--no-cache]" >&2
        exit 1
        ;;
    esac
done

if [ ${#exp_names[@]} -eq 0 ]; then
    exp_names=("ci")
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

run_test() {
    local exp_name="$1"
    echo "--------------------------------------------------"
    echo "Running test for experiment: $exp_name"
    local out_dir="$exp_dir/out/$exp_name"
    echo "Cleaning previous experiment output at $out_dir"
    rm -rf "$out_dir"

    echo "Running synthetic experiment: $exp_name"
    cd "$repo_root"
    python -m exp_runner.runner run synthetic "$exp_name" $no_cache --smoke-test --plot

    echo "Test for $exp_name passed."
}

for exp in "${exp_names[@]}"; do
    run_test "$exp"
done

echo "All synthetic CI experiment tests passed."
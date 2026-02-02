#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/socialnet"
exp_name="ci"
config_dir="$exp_dir/data/in/$exp_name"
gen_config="$config_dir/gen_config.json"
policies_file="$config_dir/policies"
out_dir="$exp_dir/data/out/$exp_name"
no_cache=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
    --no-cache)
        no_cache="--no-cache"
        shift 1
        ;;
    *)
        echo "Unknown argument: $1" >&2
        echo "Usage: $0 [--no-cache]" >&2
        exit 1
        ;;
    esac
done

if [ ! -d "$exp_dir" ]; then
    echo "Socialnet experiment directory not found at $exp_dir" >&2
    exit 1
fi

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

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker is required to run the socialnet experiment test." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "Cargo is required to run the socialnet experiment test." >&2
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

echo "Running socialnet experiment: $exp_name"
cd "$repo_root"
python -m exp.runner run socialnet "$exp_name" $no_cache --smoke-test --plot

echo "Socialnet experiment test passed."
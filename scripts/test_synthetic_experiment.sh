#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exp_dir="$repo_root/exp/synthetic"
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
    echo "Synthetic experiment directory not found at $exp_dir" >&2
    exit 1
fi

if [ ! -d "$config_dir" ]; then
    echo "CI experiment config not found at $config_dir" >&2
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

echo "Cleaning previous experiment output at $out_dir"
rm -rf "$out_dir"

echo "Running synthetic experiment: $exp_name"
cd "$repo_root"
./exp/synthetic/scripts/run-experiment.sh "$exp_name" $no_cache

assert_path_exists() {
    if [ -e "$1" ]; then
        echo "File exists: $1"
    else
        echo "Expected path missing: $1" >&2
        exit 1
    fi
}

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

echo "Synthetic CI experiment test passed."
